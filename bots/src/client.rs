use crate::run::TrafficCounters;
use anyhow::{Context, Result, bail};
use futures_util::{SinkExt, StreamExt};
use std::time::{Duration, Instant};
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
const BOT_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(30);

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
    connect_bot_with_timeout(
        world_url,
        presence_url,
        origin,
        subprotocol,
        auth_token,
        identity,
        BOT_HANDSHAKE_TIMEOUT,
    )
    .await
}

async fn connect_bot_with_timeout(
    world_url: &str,
    presence_url: &str,
    origin: &str,
    subprotocol: &str,
    auth_token: &str,
    identity: PlayerIdentity,
    handshake_timeout: Duration,
) -> Result<ConnectedBot> {
    tokio::time::timeout(
        handshake_timeout,
        connect_bot_inner(
            world_url,
            presence_url,
            origin,
            subprotocol,
            auth_token,
            identity,
        ),
    )
    .await
    .with_context(|| {
        format!(
            "bot connection handshake timed out after {} ms",
            handshake_timeout.as_millis()
        )
    })?
}

async fn connect_bot_inner(
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
    let opened_bytes = next_binary(&mut world, "world server").await?;
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
    let presence_opened_bytes = next_binary(&mut presence, "presence server").await?;
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

pub(crate) enum BotSocketMessage<'a> {
    Binary(&'a [u8]),
    Close(String),
    Control,
}

pub(crate) fn classify_bot_socket_message<'a>(
    message: &'a Message,
    peer: &str,
) -> Result<BotSocketMessage<'a>> {
    match message {
        Message::Binary(bytes) => Ok(BotSocketMessage::Binary(bytes)),
        Message::Close(frame) => Ok(BotSocketMessage::Close(format!("{frame:?}"))),
        Message::Ping(_) | Message::Pong(_) => Ok(BotSocketMessage::Control),
        Message::Text(_) => bail!("{peer} sent text data; VXWP is binary-only"),
        Message::Frame(_) => bail!("{peer} delivered an unexpected raw WebSocket frame"),
    }
}

async fn next_binary(socket: &mut BotSocket, peer: &str) -> Result<Vec<u8>> {
    while let Some(message) = socket.next().await {
        let message = message?;
        match classify_bot_socket_message(&message, peer)? {
            BotSocketMessage::Binary(bytes) => return Ok(bytes.to_vec()),
            BotSocketMessage::Close(frame) => {
                bail!("server closed during handshake: {frame}");
            }
            BotSocketMessage::Control => {}
        }
    }
    bail!("server ended WebSocket during handshake")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;
    use tokio_tungstenite::accept_hdr_async;
    use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};
    use voxels_world::protocol::{BrowserUserId, PlayerId};

    #[test]
    fn websocket_data_messages_are_binary_only() {
        let text = Message::Text("not VXWP".into());
        let error = match classify_bot_socket_message(&text, "world server") {
            Err(error) => error,
            Ok(_) => panic!("text data must be rejected"),
        };
        assert_eq!(
            error.to_string(),
            "world server sent text data; VXWP is binary-only"
        );

        assert!(matches!(
            classify_bot_socket_message(&Message::Ping(Vec::new().into()), "world server"),
            Ok(BotSocketMessage::Control)
        ));
        assert!(matches!(
            classify_bot_socket_message(&Message::Pong(Vec::new().into()), "world server"),
            Ok(BotSocketMessage::Control)
        ));
    }

    #[tokio::test]
    async fn text_handshake_message_fails_without_waiting_for_timeout() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let text_server = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            let mut websocket = accept_hdr_async(
                socket,
                |_request: &Request, mut response: Response| {
                    response.headers_mut().insert(
                        SEC_WEBSOCKET_PROTOCOL,
                        HeaderValue::from_static("voxels.v1"),
                    );
                    Ok(response)
                },
            )
            .await
            .unwrap();
            websocket
                .send(Message::Text("not VXWP".into()))
                .await
                .unwrap();
        });
        let url = format!("ws://{address}/world");
        let identity = PlayerIdentity {
            browser_user_id: BrowserUserId::from_bytes([1; 16]),
            player_id: PlayerId::from_bytes([2; 16]),
            player_name: "text-test".to_owned(),
        };

        let result = connect_bot_with_timeout(
            &url,
            &url,
            "http://127.0.0.1",
            "voxels.v1",
            "test-token",
            identity,
            Duration::from_secs(1),
        )
        .await;
        let Err(error) = result else {
            panic!("text handshake data must be rejected");
        };

        assert!(format!("{error:#}").contains("VXWP is binary-only"));
        text_server.await.unwrap();
    }

    #[tokio::test]
    async fn silent_endpoint_cannot_stall_the_bot_population() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let silent_server = tokio::spawn(async move {
            let (_socket, _) = listener.accept().await.unwrap();
            std::future::pending::<()>().await;
        });
        let url = format!("ws://{address}/world");
        let identity = PlayerIdentity {
            browser_user_id: BrowserUserId::from_bytes([1; 16]),
            player_id: PlayerId::from_bytes([2; 16]),
            player_name: "timeout-test".to_owned(),
        };

        let result = connect_bot_with_timeout(
            &url,
            &url,
            "http://127.0.0.1",
            "voxels.v1",
            "test-token",
            identity,
            Duration::from_millis(25),
        )
        .await;
        let Err(error) = result else {
            panic!("silent endpoint must hit the bounded handshake deadline");
        };

        assert!(format!("{error:#}").contains("timed out after 25 ms"));
        silent_server.abort();
    }
}
