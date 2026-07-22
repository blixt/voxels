use crate::run::TrafficCounters;
use anyhow::{Context, Result, bail};
use futures_util::{SinkExt, StreamExt};
use std::time::Instant;
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::header::{ORIGIN, SEC_WEBSOCKET_PROTOCOL};
use tokio_tungstenite::tungstenite::http::{HeaderValue, StatusCode};
use tokio_tungstenite::tungstenite::protocol::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async};
use voxels_world::protocol::{
    OpenPresence, OpenWorld, PlayerIdentity, WorldCapabilities, WorldOpened,
    decode_presence_opened, decode_world_opened, encode_open_presence, encode_open_world,
};

pub type BotSocket = WebSocketStream<MaybeTlsStream<TcpStream>>;

pub struct ConnectedBot {
    pub world: BotSocket,
    pub presence: BotSocket,
    pub opened: WorldOpened,
    pub handshake_ms: f64,
    pub traffic: TrafficCounters,
}

pub async fn connect_bot(
    world_url: &str,
    presence_url: &str,
    origin: &str,
    subprotocol: &str,
    auth_token: &str,
    identity: PlayerIdentity,
) -> Result<ConnectedBot> {
    let started = Instant::now();
    let mut traffic = TrafficCounters::default();
    let mut world = connect_socket(world_url, origin, subprotocol, auth_token)
        .await
        .context("connect world socket")?;
    let open_world = encode_open_world(&OpenWorld {
        max_in_flight_batches: 16,
        identity: identity.clone(),
    })?;
    traffic.sent(&open_world)?;
    world.send(Message::Binary(open_world.into())).await?;
    let opened_bytes = next_binary(&mut world).await?;
    traffic.received(&opened_bytes)?;
    let opened = decode_world_opened(&opened_bytes)?;
    opened
        .manifest
        .validate()
        .context("server returned an invalid world manifest")?;
    if opened.identity != identity {
        bail!("world server echoed a different player identity");
    }
    let required_capabilities = WorldCapabilities::CANONICAL_CHUNKS
        .union(WorldCapabilities::SURFACE_LOD)
        .union(WorldCapabilities::PLAYER_PRESENCE)
        .union(WorldCapabilities::SERVER_EDITS);
    if !opened.capabilities.contains(required_capabilities) {
        bail!("world server lacks a capability required by native bots");
    }

    let mut presence = connect_socket(presence_url, origin, subprotocol, auth_token)
        .await
        .context("connect presence socket")?;
    let open_presence = encode_open_presence(OpenPresence {
        session_id: opened.presence_session_id,
    })?;
    traffic.sent(&open_presence)?;
    presence.send(Message::Binary(open_presence.into())).await?;
    let presence_opened_bytes = next_binary(&mut presence).await?;
    traffic.received(&presence_opened_bytes)?;
    let presence_opened = decode_presence_opened(&presence_opened_bytes)?;
    if presence_opened.connection_id != opened.connection_id {
        bail!("presence attached to a different world connection");
    }

    Ok(ConnectedBot {
        world,
        presence,
        opened,
        handshake_ms: started.elapsed().as_secs_f64() * 1_000.0,
        traffic,
    })
}

async fn connect_socket(
    url: &str,
    origin: &str,
    subprotocol: &str,
    auth_token: &str,
) -> Result<BotSocket> {
    let mut request = url.into_client_request()?;
    request
        .headers_mut()
        .insert(ORIGIN, HeaderValue::from_str(origin)?);
    request.headers_mut().insert(
        SEC_WEBSOCKET_PROTOCOL,
        HeaderValue::from_str(&format!("{subprotocol}, {auth_token}"))?,
    );
    let (socket, response) = connect_async(request).await?;
    if response.status() != StatusCode::SWITCHING_PROTOCOLS {
        bail!("WebSocket upgrade returned {}", response.status());
    }
    let negotiated = response
        .headers()
        .get(SEC_WEBSOCKET_PROTOCOL)
        .and_then(|value| value.to_str().ok());
    if negotiated != Some(subprotocol) {
        bail!("server negotiated unexpected WebSocket subprotocol");
    }
    Ok(socket)
}

async fn next_binary(socket: &mut BotSocket) -> Result<Vec<u8>> {
    while let Some(message) = socket.next().await {
        match message? {
            Message::Binary(bytes) => return Ok(bytes.to_vec()),
            Message::Close(frame) => {
                bail!("server closed during handshake: {frame:?}");
            }
            _ => {}
        }
    }
    bail!("server ended WebSocket during handshake")
}
