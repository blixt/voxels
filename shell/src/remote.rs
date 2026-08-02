//! Browser-worker client for the versioned native world-service protocol.
//!
//! This module owns WebSocket lifecycle and request correlation only. It never names a world
//! provider: provider identity arrives in [`WorldOpened`] and every decoded product is checked
//! against that negotiated identity before it reaches the engine.

use js_sys::{Array, ArrayBuffer, Date, Uint8Array};
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll, Waker};
use voxels_client_config::WorldTransportConfig;
use voxels_runtime::{WorkStage, WorkTicket};
use voxels_world::protocol::{
    self, ChunkBatchRequest, ChunkBatchResult, EditAction, EditCommand, EditCommit,
    FrameReassembler, OpenWorld, PlayerIdentity, TerrainDirectoryBatchRequest,
    TerrainDirectoryBatchResult, TerrainRegionColumnBatchRequest, TerrainRegionColumnBatchResult,
    VirtualTerrainPageBatchRequest, VirtualTerrainPageBatchResult, WorldCapabilities, WorldOpened,
};
use voxels_world::{
    ChunkCoord, TerrainPageBatchRequestV1, TerrainPageTransferIdentity, WorldManifestHash,
    WorldProductPriority, WorldSourceError, WorldSourceIdentityHash,
};
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
use web_sys::{BinaryType, CloseEvent, Event, MessageEvent, WebSocket, WorkerGlobalScope};

use crate::request_window::priority_preemption_candidate;

pub type RemoteRequestId = u64;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RemoteConnectionState {
    Connecting,
    Handshaking,
    Open,
    WaitingToReconnect,
    Failed,
    Closed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RemoteWorldError {
    InvalidConfig(String),
    Socket(String),
    NotOpen,
    Backpressured,
    RequestWindowFull,
    InvalidBatch(&'static str),
    Protocol(String),
    ResponseMismatch(&'static str),
    Server(String),
    Source(WorldSourceError),
    TimedOut,
    Preempted,
    Disconnected(String),
    Canceled,
    Closed,
}

impl std::fmt::Display for RemoteWorldError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidConfig(reason) => {
                write!(formatter, "invalid remote world config: {reason}")
            }
            Self::Socket(reason) => write!(formatter, "world WebSocket failed: {reason}"),
            Self::NotOpen => formatter.write_str("world service handshake is not open"),
            Self::Backpressured => {
                formatter.write_str("world WebSocket is above its configured buffer watermark")
            }
            Self::RequestWindowFull => formatter.write_str("world request window is full"),
            Self::InvalidBatch(reason) => {
                write!(formatter, "invalid world product batch: {reason}")
            }
            Self::Protocol(reason) => write!(formatter, "world protocol error: {reason}"),
            Self::ResponseMismatch(reason) => {
                write!(formatter, "world response mismatch: {reason}")
            }
            Self::Server(reason) => write!(formatter, "world service error: {reason}"),
            Self::Source(error) => error.fmt(formatter),
            Self::TimedOut => formatter.write_str("world request timed out"),
            Self::Preempted => {
                formatter.write_str("world request was preempted by collision-critical work")
            }
            Self::Disconnected(reason) => write!(formatter, "world service disconnected: {reason}"),
            Self::Canceled => formatter.write_str("world request was canceled"),
            Self::Closed => formatter.write_str("remote world client is closed"),
        }
    }
}

impl std::error::Error for RemoteWorldError {}

#[derive(Clone, Debug)]
pub struct RemoteChunkCompletion {
    pub request_id: RemoteRequestId,
    pub tickets: Vec<WorkTicket>,
    pub result: Result<ChunkBatchResult, RemoteWorldError>,
}

#[derive(Clone, Debug)]
pub struct RemoteTerrainDirectoryCompletion {
    pub request_id: RemoteRequestId,
    pub roots: Vec<voxels_world::TerrainPageKey>,
    pub result: Result<TerrainDirectoryBatchResult, RemoteWorldError>,
}

#[derive(Clone, Debug)]
pub struct RemoteTerrainRegionColumnCompletion {
    pub request_id: RemoteRequestId,
    pub columns: Vec<[i32; 2]>,
    pub result: Result<TerrainRegionColumnBatchResult, RemoteWorldError>,
}

#[derive(Clone, Debug)]
pub struct RemoteTerrainPageCompletion {
    pub request_id: RemoteRequestId,
    pub requested: Vec<TerrainPageTransferIdentity>,
    pub result: Result<VirtualTerrainPageBatchResult, RemoteWorldError>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RemoteEditEvent {
    Commit(Box<EditCommit>),
    ResyncRequired { revision: u64 },
    Rejected { operation_id: u64, message: String },
}

#[derive(Clone)]
pub struct RemoteWorldClient {
    inner: Rc<RemoteInner>,
}

impl RemoteWorldClient {
    /// Opens the socket and asynchronously completes the versioned `OpenWorld` handshake.
    pub async fn connect(
        config: WorldTransportConfig,
        identity: PlayerIdentity,
    ) -> Result<Self, RemoteWorldError> {
        let client = Self::start(config, identity)?;
        client.wait_until_open().await?;
        Ok(client)
    }

    /// Starts connecting without awaiting the handshake. Use this when startup has other work to
    /// overlap, then await [`Self::wait_until_open`] before submitting products.
    pub fn start(
        config: WorldTransportConfig,
        identity: PlayerIdentity,
    ) -> Result<Self, RemoteWorldError> {
        config
            .validate()
            .map_err(|error| RemoteWorldError::InvalidConfig(error.to_string()))?;
        identity
            .validate()
            .map_err(|error| RemoteWorldError::InvalidConfig(error.to_owned()))?;
        let inner = Rc::new(RemoteInner::new(config, identity));
        RemoteInner::open_socket(&inner)?;
        Ok(Self { inner })
    }

    pub async fn wait_until_open(&self) -> Result<WorldOpened, RemoteWorldError> {
        if let Some(opened) = self.world_opened() {
            return Ok(opened);
        }
        if let Some(error) = self.inner.terminal_error() {
            return Err(error);
        }
        let (sender, receiver) = local_channel();
        self.inner.open_waiters.borrow_mut().push(sender);
        receiver.await
    }

    pub fn world_opened(&self) -> Option<WorldOpened> {
        self.inner.opened.borrow().clone()
    }

    pub fn source_identity_hash(&self) -> Option<WorldSourceIdentityHash> {
        self.inner
            .opened
            .borrow()
            .as_ref()
            .map(|opened| opened.manifest.source_identity_hash())
    }

    /// Submits scheduler generation capabilities and returns their protocol request id.
    pub fn submit_chunk_batch(
        &self,
        priority: WorldProductPriority,
        tickets: Vec<WorkTicket>,
    ) -> Result<RemoteRequestId, RemoteWorldError> {
        if tickets
            .iter()
            .any(|ticket| ticket.stage != WorkStage::Generation)
        {
            return Err(RemoteWorldError::InvalidBatch(
                "all work tickets must belong to the generation stage",
            ));
        }
        let coords = tickets.iter().map(|ticket| ticket.coord).collect();
        self.inner
            .send_chunk_request(priority, coords, ChunkDelivery::Drain(tickets))
    }

    /// One-shot bootstrap path for camera restoration and other work before the frame pump starts.
    pub async fn request_chunks(
        &self,
        priority: WorldProductPriority,
        coords: Vec<ChunkCoord>,
    ) -> Result<ChunkBatchResult, RemoteWorldError> {
        let (sender, receiver) = local_channel();
        self.inner
            .send_chunk_request(priority, coords, ChunkDelivery::OneShot(sender))?;
        receiver.await
    }

    /// Removes a request locally before emitting best-effort cancellation. A late response for the
    /// id is intentionally discarded and cannot complete a reused scheduler ticket.
    pub fn cancel(&self, request_id: RemoteRequestId) -> bool {
        self.inner.cancel(request_id)
    }

    pub fn next_completion(&self) -> Option<RemoteChunkCompletion> {
        self.inner.completions.borrow_mut().pop_front()
    }

    /// Cancels scheduler-owned chunk batches once none of their coordinates remain desired.
    ///
    /// One-shot bootstrap requests are deliberately excluded: they are not backed by scheduler
    /// capabilities and their caller owns their lifetime directly. Mixed scheduler batches remain
    /// alive so a useful coordinate is not discarded with stale siblings.
    pub fn cancel_chunk_batches_outside(&self, keep: impl Fn(ChunkCoord) -> bool) -> usize {
        self.inner.cancel_chunk_batches_outside(keep)
    }

    pub fn submit_terrain_directory_batch(
        &self,
        priority: WorldProductPriority,
        roots: Vec<voxels_world::TerrainPageKey>,
    ) -> Result<RemoteRequestId, RemoteWorldError> {
        self.inner.send_terrain_directory_request(priority, roots)
    }

    pub fn next_terrain_directory_completion(&self) -> Option<RemoteTerrainDirectoryCompletion> {
        self.inner
            .terrain_directory_completions
            .borrow_mut()
            .pop_front()
    }

    pub fn submit_terrain_region_column_batch(
        &self,
        priority: WorldProductPriority,
        columns: Vec<[i32; 2]>,
    ) -> Result<RemoteRequestId, RemoteWorldError> {
        self.inner
            .send_terrain_region_column_request(priority, columns)
    }

    pub fn next_terrain_region_column_completion(
        &self,
    ) -> Option<RemoteTerrainRegionColumnCompletion> {
        self.inner
            .terrain_region_column_completions
            .borrow_mut()
            .pop_front()
    }

    pub fn submit_terrain_page_batch(
        &self,
        priority: WorldProductPriority,
        pages: Vec<TerrainPageTransferIdentity>,
    ) -> Result<RemoteRequestId, RemoteWorldError> {
        self.inner.send_terrain_page_request(priority, pages)
    }

    /// Reports whether a batch at this priority can claim the negotiated request window.
    ///
    /// Schedulers use this before transferring pending work into their in-flight state. The send
    /// path still performs the authoritative check, including priority preemption and socket
    /// backpressure, but a full equal-priority window is normal flow control rather than a failed
    /// terrain request that should consume retry budget.
    pub fn has_request_capacity(&self, priority: WorldProductPriority) -> bool {
        self.inner.has_request_window(priority)
    }

    pub fn next_terrain_page_completion(&self) -> Option<RemoteTerrainPageCompletion> {
        self.inner.terrain_page_completions.borrow_mut().pop_front()
    }

    /// Cancels page batches once none of their identities remain in the current immutable target.
    ///
    /// Mixed batches stay alive so a still-required coherent sibling is not discarded with stale
    /// work. The generated `Canceled` completions return scheduler capabilities without retry
    /// backoff, allowing newly exposed ownership pages to use the same request window immediately.
    pub fn cancel_terrain_page_batches_outside(
        &self,
        keep: impl Fn(TerrainPageTransferIdentity) -> bool,
    ) -> usize {
        self.inner.cancel_terrain_page_batches_outside(keep)
    }

    pub fn submit_edit(&self, action: EditAction) -> Result<RemoteRequestId, RemoteWorldError> {
        self.inner.send_edit(action)
    }

    pub fn drain_edit_events(&self) -> Vec<RemoteEditEvent> {
        self.inner.edit_events.borrow_mut().drain(..).collect()
    }

    pub fn close(&self) {
        self.inner.close();
    }
}

impl Drop for RemoteWorldClient {
    fn drop(&mut self) {
        if Rc::strong_count(&self.inner) == 1 {
            self.inner.close();
        }
    }
}

struct RemoteInner {
    config: WorldTransportConfig,
    identity: PlayerIdentity,
    state: Cell<RemoteConnectionState>,
    socket: RefCell<Option<WebSocket>>,
    handlers: RefCell<Option<SocketHandlers>>,
    generation: Cell<u64>,
    next_request_id: Cell<u64>,
    opened: RefCell<Option<WorldOpened>>,
    pinned_manifest_hash: RefCell<Option<WorldManifestHash>>,
    open_waiters: RefCell<Vec<LocalSender<Result<WorldOpened, RemoteWorldError>>>>,
    pending: RefCell<BTreeMap<RemoteRequestId, PendingBatch>>,
    completions: RefCell<VecDeque<RemoteChunkCompletion>>,
    terrain_region_column_completions: RefCell<VecDeque<RemoteTerrainRegionColumnCompletion>>,
    terrain_directory_completions: RefCell<VecDeque<RemoteTerrainDirectoryCompletion>>,
    terrain_page_completions: RefCell<VecDeque<RemoteTerrainPageCompletion>>,
    pending_edits: RefCell<BTreeMap<u64, EditCommand>>,
    edit_events: RefCell<VecDeque<RemoteEditEvent>>,
    frame_reassembler: RefCell<FrameReassembler>,
    send_paused: Cell<bool>,
    reconnect_attempts: Cell<u32>,
    terminal_error: RefCell<Option<RemoteWorldError>>,
}

enum PendingBatch {
    Chunks {
        priority: WorldProductPriority,
        expected_coords: Vec<ChunkCoord>,
        delivery: ChunkDelivery,
    },
    TerrainDirectories {
        priority: WorldProductPriority,
        roots: Vec<voxels_world::TerrainPageKey>,
    },
    TerrainRegionColumns {
        priority: WorldProductPriority,
        columns: Vec<[i32; 2]>,
    },
    TerrainPages {
        priority: WorldProductPriority,
        requested: Vec<TerrainPageTransferIdentity>,
    },
}

#[derive(Clone, Copy)]
enum PendingBatchKind {
    Chunks,
    TerrainRegionColumns,
    TerrainDirectories,
    TerrainPages,
}

impl PendingBatch {
    const fn priority(&self) -> WorldProductPriority {
        match self {
            Self::Chunks { priority, .. }
            | Self::TerrainRegionColumns { priority, .. }
            | Self::TerrainDirectories { priority, .. }
            | Self::TerrainPages { priority, .. } => *priority,
        }
    }

    const fn kind(&self) -> PendingBatchKind {
        match self {
            Self::Chunks { .. } => PendingBatchKind::Chunks,
            Self::TerrainRegionColumns { .. } => PendingBatchKind::TerrainRegionColumns,
            Self::TerrainDirectories { .. } => PendingBatchKind::TerrainDirectories,
            Self::TerrainPages { .. } => PendingBatchKind::TerrainPages,
        }
    }
}

enum ChunkDelivery {
    Drain(Vec<WorkTicket>),
    OneShot(LocalSender<Result<ChunkBatchResult, RemoteWorldError>>),
}

struct SocketHandlers {
    _open: Closure<dyn FnMut(Event)>,
    _message: Closure<dyn FnMut(MessageEvent)>,
    _error: Closure<dyn FnMut(Event)>,
    _close: Closure<dyn FnMut(CloseEvent)>,
}

impl RemoteInner {
    fn new(config: WorldTransportConfig, identity: PlayerIdentity) -> Self {
        Self {
            config,
            identity,
            state: Cell::new(RemoteConnectionState::Connecting),
            socket: RefCell::new(None),
            handlers: RefCell::new(None),
            generation: Cell::new(0),
            next_request_id: Cell::new(1),
            opened: RefCell::new(None),
            pinned_manifest_hash: RefCell::new(None),
            open_waiters: RefCell::new(Vec::new()),
            pending: RefCell::new(BTreeMap::new()),
            completions: RefCell::new(VecDeque::new()),
            terrain_region_column_completions: RefCell::new(VecDeque::new()),
            terrain_directory_completions: RefCell::new(VecDeque::new()),
            terrain_page_completions: RefCell::new(VecDeque::new()),
            pending_edits: RefCell::new(BTreeMap::new()),
            edit_events: RefCell::new(VecDeque::new()),
            frame_reassembler: RefCell::new(FrameReassembler::default()),
            send_paused: Cell::new(false),
            reconnect_attempts: Cell::new(0),
            terminal_error: RefCell::new(None),
        }
    }

    fn open_socket(inner: &Rc<Self>) -> Result<(), RemoteWorldError> {
        if matches!(
            inner.state.get(),
            RemoteConnectionState::Closed | RemoteConnectionState::Failed
        ) {
            return Err(inner.terminal_error().unwrap_or(RemoteWorldError::Closed));
        }
        inner.detach_socket(1001, "world reconnect");
        let protocols = Array::new();
        protocols.push(&inner.config.subprotocol.clone().into());
        protocols.push(&inner.config.auth_subprotocol_token.clone().into());
        let socket = WebSocket::new_with_str_sequence(&inner.config.endpoint, protocols.as_ref())
            .map_err(|error| RemoteWorldError::Socket(js_reason(error)))?;
        socket.set_binary_type(BinaryType::Arraybuffer);
        let generation = inner.generation.get().wrapping_add(1);
        inner.generation.set(generation);
        inner.state.set(RemoteConnectionState::Connecting);
        inner.opened.borrow_mut().take();

        let weak = Rc::downgrade(inner);
        let open = Closure::new(move |_event: Event| {
            if let Some(inner) = weak.upgrade() {
                inner.handle_open(generation);
            }
        });
        let weak = Rc::downgrade(inner);
        let message = Closure::new(move |event: MessageEvent| {
            if let Some(inner) = weak.upgrade() {
                let data = event.data();
                match data.dyn_into::<ArrayBuffer>() {
                    Ok(buffer) => {
                        inner.handle_message(generation, Uint8Array::new(&buffer).to_vec())
                    }
                    Err(_) => inner.disconnect(
                        generation,
                        RemoteWorldError::Protocol(
                            "server sent a non-binary WebSocket message".to_owned(),
                        ),
                    ),
                }
            }
        });
        let weak = Rc::downgrade(inner);
        // WebSocket's `error` event is the base Event type and intentionally carries no details.
        // Reading ErrorEvent.message through an unchecked cast traps in Chrome when a handshake is
        // rejected, masking the actual reconnect path with a JS type error.
        let error = Closure::new(move |_event: Event| {
            if let Some(inner) = weak.upgrade() {
                inner.disconnect(
                    generation,
                    RemoteWorldError::Socket("WebSocket error".to_owned()),
                );
            }
        });
        let weak = Rc::downgrade(inner);
        let close = Closure::new(move |event: CloseEvent| {
            if let Some(inner) = weak.upgrade() {
                inner.disconnect(
                    generation,
                    RemoteWorldError::Disconnected(format!(
                        "code {}{}",
                        event.code(),
                        if event.reason().is_empty() {
                            String::new()
                        } else {
                            format!(": {}", event.reason())
                        }
                    )),
                );
            }
        });
        socket.set_onopen(Some(open.as_ref().unchecked_ref()));
        socket.set_onmessage(Some(message.as_ref().unchecked_ref()));
        socket.set_onerror(Some(error.as_ref().unchecked_ref()));
        socket.set_onclose(Some(close.as_ref().unchecked_ref()));
        *inner.socket.borrow_mut() = Some(socket);
        *inner.handlers.borrow_mut() = Some(SocketHandlers {
            _open: open,
            _message: message,
            _error: error,
            _close: close,
        });

        let weak = Rc::downgrade(inner);
        schedule_after(inner.config.request_timeout_ms, move || {
            if let Some(inner) = weak.upgrade()
                && inner.generation.get() == generation
                && matches!(
                    inner.state.get(),
                    RemoteConnectionState::Connecting | RemoteConnectionState::Handshaking
                )
            {
                inner.disconnect(generation, RemoteWorldError::TimedOut);
            }
        })?;
        Ok(())
    }

    fn handle_open(self: &Rc<Self>, generation: u64) {
        if generation != self.generation.get()
            || self.state.get() != RemoteConnectionState::Connecting
        {
            return;
        }
        let Some(socket) = self.socket.borrow().clone() else {
            return;
        };
        if socket.protocol() != self.config.subprotocol {
            self.disconnect(
                generation,
                RemoteWorldError::Protocol(
                    "server did not negotiate the configured subprotocol".to_owned(),
                ),
            );
            return;
        }
        let requested_window = u16::try_from(self.config.max_in_flight_batches).unwrap_or(u16::MAX);
        let frame = match protocol::encode_open_world(&OpenWorld {
            max_in_flight_batches: requested_window,
            identity: self.identity.clone(),
        }) {
            Ok(frame) => frame,
            Err(error) => {
                self.disconnect(generation, RemoteWorldError::Protocol(error.to_string()));
                return;
            }
        };
        if let Err(error) = socket.send_with_u8_array(&frame) {
            self.disconnect(generation, RemoteWorldError::Socket(js_reason(error)));
            return;
        }
        self.state.set(RemoteConnectionState::Handshaking);
    }

    fn handle_message(self: &Rc<Self>, generation: u64, bytes: Vec<u8>) {
        if generation != self.generation.get()
            || matches!(
                self.state.get(),
                RemoteConnectionState::Closed | RemoteConnectionState::Failed
            )
        {
            return;
        }
        let kind = match protocol::message_kind(&bytes) {
            Ok(kind) => kind,
            Err(error) => {
                self.disconnect(generation, RemoteWorldError::Protocol(error.to_string()));
                return;
            }
        };
        if kind == protocol::frame_fragment_kind() {
            let completed = self.frame_reassembler.borrow_mut().accept(&bytes);
            match completed {
                Ok(Some(completed)) => self.handle_message(generation, completed),
                Ok(None) => {}
                Err(error) => {
                    self.disconnect(generation, RemoteWorldError::Protocol(error.to_string()))
                }
            }
        } else if kind == protocol::frame_fragment_abort_kind() {
            match protocol::decode_frame_fragment_abort(&bytes) {
                Ok(transfer_id) => {
                    self.frame_reassembler.borrow_mut().abort(transfer_id);
                }
                Err(error) => {
                    self.disconnect(generation, RemoteWorldError::Protocol(error.to_string()));
                }
            }
        } else if kind == protocol::world_opened_kind() {
            self.handle_world_opened(generation, &bytes);
        } else if kind == protocol::chunk_batch_result_kind() {
            self.handle_chunk_result(generation, &bytes);
        } else if kind == protocol::terrain_region_column_batch_result_kind() {
            self.handle_terrain_region_column_result(generation, &bytes);
        } else if kind == protocol::terrain_directory_batch_result_kind() {
            self.handle_terrain_directory_result(generation, &bytes);
        } else if kind == protocol::virtual_terrain_page_batch_result_kind() {
            self.handle_terrain_page_result(generation, &bytes);
        } else if kind == protocol::edit_commit_kind() {
            self.handle_edit_commit(generation, &bytes);
        } else if kind == protocol::resync_required_kind() {
            self.handle_resync_required(generation, &bytes);
        } else if kind == protocol::error_kind() {
            self.handle_server_error(generation, &bytes);
        } else {
            self.disconnect(
                generation,
                RemoteWorldError::Protocol(format!("unexpected server message kind {kind}")),
            );
        }
    }

    fn handle_world_opened(self: &Rc<Self>, generation: u64, bytes: &[u8]) {
        if self.state.get() != RemoteConnectionState::Handshaking {
            self.disconnect(
                generation,
                RemoteWorldError::Protocol("unexpected duplicate WorldOpened".to_owned()),
            );
            return;
        }
        let opened = match protocol::decode_world_opened(bytes) {
            Ok(opened) => opened,
            Err(error) => {
                self.disconnect(generation, RemoteWorldError::Protocol(error.to_string()));
                return;
            }
        };
        if let Err(error) = opened.manifest.validate() {
            self.disconnect(generation, RemoteWorldError::Protocol(error.to_string()));
            return;
        }
        if opened.identity != self.identity {
            self.disconnect(
                generation,
                RemoteWorldError::ResponseMismatch("server echoed a different player identity"),
            );
            return;
        }
        if !opened
            .capabilities
            .contains(WorldCapabilities::CANONICAL_CHUNKS)
            || !opened
                .capabilities
                .contains(WorldCapabilities::VIRTUAL_TERRAIN)
            || !opened
                .capabilities
                .contains(WorldCapabilities::PLAYER_PRESENCE)
            || !opened
                .capabilities
                .contains(WorldCapabilities::SERVER_EDITS)
        {
            self.disconnect(
                generation,
                RemoteWorldError::Protocol(
                    "world service lacks a required world or player-presence capability".to_owned(),
                ),
            );
            return;
        }
        let manifest_hash = match opened.manifest.manifest_hash() {
            Ok(hash) => hash,
            Err(error) => {
                self.disconnect(generation, RemoteWorldError::Protocol(error.to_string()));
                return;
            }
        };
        if self
            .pinned_manifest_hash
            .borrow()
            .is_some_and(|pinned| pinned != manifest_hash)
        {
            self.reconnect_attempts
                .set(self.config.reconnect_attempt_limit);
            self.disconnect(
                generation,
                RemoteWorldError::Protocol(
                    "world manifest changed while reconnecting; refusing mixed-world data"
                        .to_owned(),
                ),
            );
            return;
        }
        *self.pinned_manifest_hash.borrow_mut() = Some(manifest_hash);
        *self.opened.borrow_mut() = Some(opened.clone());
        self.state.set(RemoteConnectionState::Open);
        self.reconnect_attempts.set(0);
        self.send_paused.set(false);
        let retry_frames = self
            .pending_edits
            .borrow()
            .values()
            .copied()
            .map(protocol::encode_edit_command)
            .collect::<Result<Vec<_>, _>>();
        let retry_frames = match retry_frames {
            Ok(frames) => frames,
            Err(error) => {
                self.disconnect(generation, RemoteWorldError::Protocol(error.to_string()));
                return;
            }
        };
        let Some(socket) = self.socket.borrow().clone() else {
            self.disconnect(generation, RemoteWorldError::NotOpen);
            return;
        };
        for frame in retry_frames {
            if let Err(error) = socket.send_with_u8_array(&frame) {
                self.disconnect(generation, RemoteWorldError::Socket(js_reason(error)));
                return;
            }
        }
        for waiter in self.open_waiters.borrow_mut().drain(..) {
            waiter.send(Ok(opened.clone()));
        }
    }

    fn handle_chunk_result(self: &Rc<Self>, generation: u64, bytes: &[u8]) {
        if self.state.get() != RemoteConnectionState::Open {
            self.disconnect(
                generation,
                RemoteWorldError::Protocol("chunk result arrived before WorldOpened".to_owned()),
            );
            return;
        }
        let result = match protocol::decode_chunk_batch_result(bytes) {
            Ok(result) => result,
            Err(error) => {
                self.disconnect(generation, RemoteWorldError::Protocol(error.to_string()));
                return;
            }
        };
        let expected = self
            .pending
            .borrow()
            .get(&result.request_id)
            .and_then(|pending| match pending {
                PendingBatch::Chunks {
                    expected_coords, ..
                } => Some(expected_coords.clone()),
                _ => None,
            });
        let Some(expected) = expected else {
            // Canceled, timed-out, and prior-connection request ids stay retired. Their late
            // responses must never be attached to current scheduler capabilities.
            return;
        };
        let expected_identity = self
            .opened
            .borrow()
            .as_ref()
            .map(|opened| opened.manifest.source_identity_hash());
        let validation = validate_chunk_result(&result, &expected, expected_identity);
        if let Err(error) = validation {
            self.finish_request(result.request_id, Err(error));
            return;
        }
        self.finish_request(result.request_id, Ok(result));
    }

    fn handle_terrain_directory_result(self: &Rc<Self>, generation: u64, bytes: &[u8]) {
        if self.state.get() != RemoteConnectionState::Open {
            self.disconnect(
                generation,
                RemoteWorldError::Protocol(
                    "terrain directory result arrived before WorldOpened".to_owned(),
                ),
            );
            return;
        }
        let result = match protocol::decode_terrain_directory_batch_result(bytes) {
            Ok(result) => result,
            Err(error) => {
                self.disconnect(generation, RemoteWorldError::Protocol(error.to_string()));
                return;
            }
        };
        let expected = self
            .pending
            .borrow()
            .get(&result.request_id)
            .and_then(|pending| match pending {
                PendingBatch::TerrainDirectories { roots, .. } => Some(roots.clone()),
                _ => None,
            });
        let Some(expected) = expected else {
            return;
        };
        let expected_identity = self
            .opened
            .borrow()
            .as_ref()
            .map(|opened| opened.manifest.source_identity_hash());
        let validation = validate_terrain_directory_result(&result, &expected, expected_identity);
        self.finish_terrain_directory_request(result.request_id, validation.map(|()| result));
    }

    fn handle_terrain_region_column_result(self: &Rc<Self>, generation: u64, bytes: &[u8]) {
        if self.state.get() != RemoteConnectionState::Open {
            self.disconnect(
                generation,
                RemoteWorldError::Protocol(
                    "terrain region column result arrived before WorldOpened".to_owned(),
                ),
            );
            return;
        }
        let result = match protocol::decode_terrain_region_column_batch_result(bytes) {
            Ok(result) => result,
            Err(error) => {
                self.disconnect(generation, RemoteWorldError::Protocol(error.to_string()));
                return;
            }
        };
        let expected = self
            .pending
            .borrow()
            .get(&result.request_id)
            .and_then(|pending| match pending {
                PendingBatch::TerrainRegionColumns { columns, .. } => Some(columns.clone()),
                _ => None,
            });
        let Some(expected) = expected else {
            return;
        };
        let expected_identity = self
            .opened
            .borrow()
            .as_ref()
            .map(|opened| opened.manifest.source_identity_hash());
        let validation =
            validate_terrain_region_column_result(&result, &expected, expected_identity);
        self.finish_terrain_region_column_request(result.request_id, validation.map(|()| result));
    }

    fn handle_terrain_page_result(self: &Rc<Self>, generation: u64, bytes: &[u8]) {
        if self.state.get() != RemoteConnectionState::Open {
            self.disconnect(
                generation,
                RemoteWorldError::Protocol(
                    "terrain page result arrived before WorldOpened".to_owned(),
                ),
            );
            return;
        }
        let Some(expected_identity) = self
            .opened
            .borrow()
            .as_ref()
            .map(|opened| opened.manifest.source_identity_hash())
        else {
            self.disconnect(generation, RemoteWorldError::NotOpen);
            return;
        };
        let result =
            match protocol::decode_virtual_terrain_page_batch_result(bytes, expected_identity) {
                Ok(result) => result,
                Err(error) => {
                    self.disconnect(generation, RemoteWorldError::Protocol(error.to_string()));
                    return;
                }
            };
        let expected = self
            .pending
            .borrow()
            .get(&result.request_id)
            .and_then(|pending| match pending {
                PendingBatch::TerrainPages { requested, .. } => Some(requested.clone()),
                _ => None,
            });
        let Some(expected) = expected else {
            return;
        };
        let validation = validate_terrain_page_result(&result, &expected, expected_identity);
        self.finish_terrain_page_request(result.request_id, validation.map(|()| result));
    }

    fn handle_edit_commit(self: &Rc<Self>, generation: u64, bytes: &[u8]) {
        if self.state.get() != RemoteConnectionState::Open {
            self.disconnect(
                generation,
                RemoteWorldError::Protocol("edit commit arrived before WorldOpened".to_owned()),
            );
            return;
        }
        let commit = match protocol::decode_edit_commit(bytes) {
            Ok(commit) => commit,
            Err(error) => {
                self.disconnect(generation, RemoteWorldError::Protocol(error.to_string()));
                return;
            }
        };
        let matches_pending = self
            .pending_edits
            .borrow()
            .get(&commit.operation_id)
            .is_some_and(|command| {
                self.opened
                    .borrow()
                    .as_ref()
                    .is_some_and(|opened| opened.connection_id == commit.editor_connection_id)
                    && command.edit_session_id == commit.edit_session_id
            });
        if matches_pending {
            self.pending_edits.borrow_mut().remove(&commit.operation_id);
        }
        self.edit_events
            .borrow_mut()
            .push_back(RemoteEditEvent::Commit(Box::new(commit)));
    }

    fn handle_resync_required(self: &Rc<Self>, generation: u64, bytes: &[u8]) {
        if self.state.get() != RemoteConnectionState::Open {
            self.disconnect(
                generation,
                RemoteWorldError::Protocol("edit resync arrived before WorldOpened".to_owned()),
            );
            return;
        }
        let resync = match protocol::decode_resync_required(bytes) {
            Ok(resync) => resync,
            Err(error) => {
                self.disconnect(generation, RemoteWorldError::Protocol(error.to_string()));
                return;
            }
        };
        self.edit_events
            .borrow_mut()
            .push_back(RemoteEditEvent::ResyncRequired {
                revision: resync.revision,
            });
    }

    fn handle_server_error(self: &Rc<Self>, generation: u64, bytes: &[u8]) {
        let (request_id, mut message) = match protocol::decode_error(bytes) {
            Ok(error) => error,
            Err(error) => {
                self.disconnect(generation, RemoteWorldError::Protocol(error.to_string()));
                return;
            }
        };
        if request_id == 0 {
            self.disconnect(generation, RemoteWorldError::Server(message));
        } else if self.pending.borrow().contains_key(&request_id) {
            self.finish_pending_error(request_id, RemoteWorldError::Server(message));
        } else {
            let command = self.pending_edits.borrow_mut().remove(&request_id);
            if let Some(command) = command {
                if message == protocol::EDIT_SESSION_NOT_CURRENT {
                    match self.reissue_edit_after_session_rotation(command) {
                        Ok(true) => return,
                        Ok(false) => {}
                        Err(error) => {
                            message = format!("{message}; automatic retry failed: {error}")
                        }
                    }
                }
                self.edit_events
                    .borrow_mut()
                    .push_back(RemoteEditEvent::Rejected {
                        operation_id: request_id,
                        message,
                    });
            }
        }
    }

    fn reissue_edit_after_session_rotation(
        self: &Rc<Self>,
        command: EditCommand,
    ) -> Result<bool, RemoteWorldError> {
        let Some(socket) = self.socket.borrow().clone() else {
            return Err(RemoteWorldError::NotOpen);
        };
        let edit_session_id = self
            .opened
            .borrow()
            .as_ref()
            .map(|opened| opened.edit_session_id)
            .ok_or(RemoteWorldError::NotOpen)?;
        let operation_id = self.allocate_request_id()?;
        let Some(reissued) = command.reissue_after_session_rotation(operation_id, edit_session_id)
        else {
            return Ok(false);
        };
        let frame = protocol::encode_edit_command(reissued)
            .map_err(|error| RemoteWorldError::Protocol(error.to_string()))?;
        socket
            .send_with_u8_array(&frame)
            .map_err(|error| RemoteWorldError::Socket(js_reason(error)))?;
        self.pending_edits
            .borrow_mut()
            .insert(operation_id, reissued);
        Ok(true)
    }

    fn send_edit(self: &Rc<Self>, action: EditAction) -> Result<RemoteRequestId, RemoteWorldError> {
        if self.state.get() != RemoteConnectionState::Open {
            return Err(self.terminal_error().unwrap_or(RemoteWorldError::NotOpen));
        }
        let Some(socket) = self.socket.borrow().clone() else {
            return Err(RemoteWorldError::NotOpen);
        };
        if socket.buffered_amount() >= self.config.buffered_amount_high_water_bytes {
            return Err(RemoteWorldError::Backpressured);
        }
        let operation_id = self.allocate_request_id()?;
        let edit_session_id = self
            .opened
            .borrow()
            .as_ref()
            .map(|opened| opened.edit_session_id)
            .ok_or(RemoteWorldError::NotOpen)?;
        let command = EditCommand {
            operation_id,
            edit_session_id,
            action,
        };
        let frame = protocol::encode_edit_command(command)
            .map_err(|error| RemoteWorldError::Protocol(error.to_string()))?;
        socket
            .send_with_u8_array(&frame)
            .map_err(|error| RemoteWorldError::Socket(js_reason(error)))?;
        self.pending_edits
            .borrow_mut()
            .insert(operation_id, command);
        Ok(operation_id)
    }

    fn send_chunk_request(
        self: &Rc<Self>,
        priority: WorldProductPriority,
        coords: Vec<ChunkCoord>,
        delivery: ChunkDelivery,
    ) -> Result<RemoteRequestId, RemoteWorldError> {
        if self.state.get() != RemoteConnectionState::Open {
            return Err(self.terminal_error().unwrap_or(RemoteWorldError::NotOpen));
        }
        if coords.is_empty() {
            return Err(RemoteWorldError::InvalidBatch("batch must not be empty"));
        }
        let unique = coords.iter().copied().collect::<BTreeSet<_>>();
        if unique.len() != coords.len() {
            return Err(RemoteWorldError::InvalidBatch(
                "batch contains duplicate chunk coordinates",
            ));
        }
        // Check transport capacity before preempting useful lower-priority work. A request that
        // cannot be sent must not consume a request-window slot or cancel its current owner.
        let socket = self.request_socket()?;
        if !self.ensure_request_window(priority) {
            return Err(RemoteWorldError::RequestWindowFull);
        }

        let request_id = self.allocate_request_id()?;
        let frame = protocol::encode_chunk_batch(&ChunkBatchRequest {
            request_id,
            priority,
            coords: coords.clone(),
        })
        .map_err(|error| RemoteWorldError::Protocol(error.to_string()))?;
        socket
            .send_with_u8_array(&frame)
            .map_err(|error| RemoteWorldError::Socket(js_reason(error)))?;
        self.pending.borrow_mut().insert(
            request_id,
            PendingBatch::Chunks {
                priority,
                expected_coords: coords,
                delivery,
            },
        );
        let weak = Rc::downgrade(self);
        if let Err(error) = schedule_after(self.config.request_timeout_ms, move || {
            if let Some(inner) = weak.upgrade() {
                inner.timeout_request(request_id);
            }
        }) {
            self.pending.borrow_mut().remove(&request_id);
            self.send_cancel(request_id);
            return Err(error);
        }
        Ok(request_id)
    }

    fn send_terrain_region_column_request(
        self: &Rc<Self>,
        priority: WorldProductPriority,
        mut columns: Vec<[i32; 2]>,
    ) -> Result<RemoteRequestId, RemoteWorldError> {
        if self.state.get() != RemoteConnectionState::Open {
            return Err(self.terminal_error().unwrap_or(RemoteWorldError::NotOpen));
        }
        columns.sort_unstable();
        protocol::encode_terrain_region_column_batch(&TerrainRegionColumnBatchRequest {
            request_id: 1,
            priority,
            columns: columns.clone(),
        })
        .map_err(|error| RemoteWorldError::Protocol(error.to_string()))?;
        let socket = self.request_socket()?;
        if !self.ensure_request_window(priority) {
            return Err(RemoteWorldError::RequestWindowFull);
        }
        let request_id = self.allocate_request_id()?;
        let frame =
            protocol::encode_terrain_region_column_batch(&TerrainRegionColumnBatchRequest {
                request_id,
                priority,
                columns: columns.clone(),
            })
            .map_err(|error| RemoteWorldError::Protocol(error.to_string()))?;
        socket
            .send_with_u8_array(&frame)
            .map_err(|error| RemoteWorldError::Socket(js_reason(error)))?;
        self.pending.borrow_mut().insert(
            request_id,
            PendingBatch::TerrainRegionColumns { priority, columns },
        );
        self.schedule_request_timeout(request_id)?;
        Ok(request_id)
    }

    fn send_terrain_directory_request(
        self: &Rc<Self>,
        priority: WorldProductPriority,
        mut roots: Vec<voxels_world::TerrainPageKey>,
    ) -> Result<RemoteRequestId, RemoteWorldError> {
        if self.state.get() != RemoteConnectionState::Open {
            return Err(self.terminal_error().unwrap_or(RemoteWorldError::NotOpen));
        }
        roots.sort_unstable();
        protocol::encode_terrain_directory_batch(&TerrainDirectoryBatchRequest {
            request_id: 1,
            priority,
            roots: roots.clone(),
        })
        .map_err(|error| RemoteWorldError::Protocol(error.to_string()))?;
        let socket = self.request_socket()?;
        if !self.ensure_request_window(priority) {
            return Err(RemoteWorldError::RequestWindowFull);
        }
        let request_id = self.allocate_request_id()?;
        let frame = protocol::encode_terrain_directory_batch(&TerrainDirectoryBatchRequest {
            request_id,
            priority,
            roots: roots.clone(),
        })
        .map_err(|error| RemoteWorldError::Protocol(error.to_string()))?;
        socket
            .send_with_u8_array(&frame)
            .map_err(|error| RemoteWorldError::Socket(js_reason(error)))?;
        self.pending.borrow_mut().insert(
            request_id,
            PendingBatch::TerrainDirectories { priority, roots },
        );
        self.schedule_request_timeout(request_id)?;
        Ok(request_id)
    }

    fn send_terrain_page_request(
        self: &Rc<Self>,
        priority: WorldProductPriority,
        mut pages: Vec<TerrainPageTransferIdentity>,
    ) -> Result<RemoteRequestId, RemoteWorldError> {
        if self.state.get() != RemoteConnectionState::Open {
            return Err(self.terminal_error().unwrap_or(RemoteWorldError::NotOpen));
        }
        let Some(source_identity_hash) = self
            .opened
            .borrow()
            .as_ref()
            .map(|opened| opened.manifest.source_identity_hash())
        else {
            return Err(RemoteWorldError::NotOpen);
        };
        pages.sort_unstable();
        protocol::encode_virtual_terrain_page_batch(&VirtualTerrainPageBatchRequest {
            request_id: 1,
            priority,
            batch: TerrainPageBatchRequestV1 {
                source_identity_hash,
                pages: pages.clone(),
            },
        })
        .map_err(|error| RemoteWorldError::Protocol(error.to_string()))?;
        let socket = self.request_socket()?;
        if !self.ensure_request_window(priority) {
            return Err(RemoteWorldError::RequestWindowFull);
        }
        let request_id = self.allocate_request_id()?;
        let frame = protocol::encode_virtual_terrain_page_batch(&VirtualTerrainPageBatchRequest {
            request_id,
            priority,
            batch: TerrainPageBatchRequestV1 {
                source_identity_hash,
                pages: pages.clone(),
            },
        })
        .map_err(|error| RemoteWorldError::Protocol(error.to_string()))?;
        socket
            .send_with_u8_array(&frame)
            .map_err(|error| RemoteWorldError::Socket(js_reason(error)))?;
        self.pending.borrow_mut().insert(
            request_id,
            PendingBatch::TerrainPages {
                priority,
                requested: pages,
            },
        );
        self.schedule_request_timeout(request_id)?;
        Ok(request_id)
    }

    fn request_socket(&self) -> Result<WebSocket, RemoteWorldError> {
        let Some(socket) = self.socket.borrow().clone() else {
            return Err(RemoteWorldError::NotOpen);
        };
        let buffered = socket.buffered_amount();
        if self.send_paused.get() {
            if buffered > self.config.buffered_amount_low_water_bytes {
                return Err(RemoteWorldError::Backpressured);
            }
            self.send_paused.set(false);
        }
        if buffered >= self.config.buffered_amount_high_water_bytes {
            self.send_paused.set(true);
            return Err(RemoteWorldError::Backpressured);
        }
        Ok(socket)
    }

    fn schedule_request_timeout(self: &Rc<Self>, request_id: u64) -> Result<(), RemoteWorldError> {
        let weak = Rc::downgrade(self);
        if let Err(error) = schedule_after(self.config.request_timeout_ms, move || {
            if let Some(inner) = weak.upgrade() {
                inner.timeout_request(request_id);
            }
        }) {
            self.pending.borrow_mut().remove(&request_id);
            self.send_cancel(request_id);
            return Err(error);
        }
        Ok(())
    }

    fn allocate_request_id(&self) -> Result<u64, RemoteWorldError> {
        let start = self.next_request_id.get().max(1);
        let mut candidate = start;
        loop {
            if !self.pending.borrow().contains_key(&candidate)
                && !self.pending_edits.borrow().contains_key(&candidate)
            {
                self.next_request_id.set(candidate.wrapping_add(1).max(1));
                return Ok(candidate);
            }
            candidate = candidate.wrapping_add(1).max(1);
            if candidate == start {
                return Err(RemoteWorldError::RequestWindowFull);
            }
        }
    }

    fn request_window(&self) -> usize {
        let server = self.opened.borrow().as_ref().map_or(1, |opened| {
            usize::from(opened.recommended_in_flight_batches)
        });
        server.min(self.config.max_in_flight_batches as usize)
    }

    fn has_request_window(&self, priority: WorldProductPriority) -> bool {
        let window = self.request_window();
        if self.pending.borrow().len() < window {
            return true;
        }
        let pending = self.pending.borrow();
        priority_preemption_candidate(
            priority,
            pending
                .iter()
                .map(|(&request_id, batch)| (request_id, batch.priority())),
        )
        .is_some()
    }

    fn ensure_request_window(&self, priority: WorldProductPriority) -> bool {
        let window = self.request_window();
        if self.pending.borrow().len() < window {
            return true;
        }
        let candidate = {
            let pending = self.pending.borrow();
            priority_preemption_candidate(
                priority,
                pending
                    .iter()
                    .map(|(&request_id, batch)| (request_id, batch.priority())),
            )
        };
        let Some(request_id) = candidate else {
            return false;
        };
        // Cancellation is emitted first on the ordered world socket. The old local capability is
        // returned to its scheduler queue before the urgent request claims the released slot.
        self.send_cancel(request_id);
        self.finish_pending_error(request_id, RemoteWorldError::Preempted);
        self.pending.borrow().len() < window
    }

    fn timeout_request(&self, request_id: u64) {
        if !self.pending.borrow().contains_key(&request_id) {
            return;
        }
        self.send_cancel(request_id);
        self.finish_pending_error(request_id, RemoteWorldError::TimedOut);
    }

    fn cancel(&self, request_id: u64) -> bool {
        if !self.pending.borrow().contains_key(&request_id) {
            return false;
        }
        self.send_cancel(request_id);
        self.finish_pending_error(request_id, RemoteWorldError::Canceled);
        true
    }

    fn cancel_chunk_batches_outside(&self, keep: impl Fn(ChunkCoord) -> bool) -> usize {
        let request_ids = self
            .pending
            .borrow()
            .iter()
            .filter_map(|(&request_id, pending)| match pending {
                PendingBatch::Chunks {
                    expected_coords,
                    delivery: ChunkDelivery::Drain(_),
                    ..
                } if expected_coords.iter().all(|coord| !keep(*coord)) => Some(request_id),
                PendingBatch::Chunks {
                    delivery: ChunkDelivery::OneShot(_),
                    ..
                }
                | PendingBatch::TerrainRegionColumns { .. }
                | PendingBatch::TerrainDirectories { .. }
                | PendingBatch::TerrainPages { .. }
                | PendingBatch::Chunks { .. } => None,
            })
            .collect::<Vec<_>>();
        let canceled = request_ids.len();
        for request_id in request_ids {
            self.cancel(request_id);
        }
        canceled
    }

    fn cancel_terrain_page_batches_outside(
        &self,
        keep: impl Fn(TerrainPageTransferIdentity) -> bool,
    ) -> usize {
        let request_ids = self
            .pending
            .borrow()
            .iter()
            .filter_map(|(&request_id, pending)| match pending {
                PendingBatch::TerrainPages { requested, .. }
                    if requested.iter().all(|identity| !keep(*identity)) =>
                {
                    Some(request_id)
                }
                PendingBatch::TerrainPages { .. }
                | PendingBatch::TerrainRegionColumns { .. }
                | PendingBatch::TerrainDirectories { .. }
                | PendingBatch::Chunks { .. } => None,
            })
            .collect::<Vec<_>>();
        let canceled = request_ids.len();
        for request_id in request_ids {
            self.cancel(request_id);
        }
        canceled
    }

    fn send_cancel(&self, request_id: u64) {
        let Ok(frame) = protocol::encode_cancel(request_id) else {
            return;
        };
        if let Some(socket) = self.socket.borrow().as_ref()
            && socket.ready_state() == WebSocket::OPEN
        {
            let _ = socket.send_with_u8_array(&frame);
        }
    }

    fn finish_request(&self, request_id: u64, result: Result<ChunkBatchResult, RemoteWorldError>) {
        let Some(pending) = self.pending.borrow_mut().remove(&request_id) else {
            return;
        };
        let PendingBatch::Chunks { delivery, .. } = pending else {
            return;
        };
        match delivery {
            ChunkDelivery::Drain(tickets) => {
                self.completions
                    .borrow_mut()
                    .push_back(RemoteChunkCompletion {
                        request_id,
                        tickets,
                        result,
                    });
            }
            ChunkDelivery::OneShot(sender) => sender.send(result),
        }
    }

    fn finish_terrain_directory_request(
        &self,
        request_id: u64,
        result: Result<TerrainDirectoryBatchResult, RemoteWorldError>,
    ) {
        let Some(pending) = self.pending.borrow_mut().remove(&request_id) else {
            return;
        };
        let PendingBatch::TerrainDirectories { roots, .. } = pending else {
            return;
        };
        self.terrain_directory_completions.borrow_mut().push_back(
            RemoteTerrainDirectoryCompletion {
                request_id,
                roots,
                result,
            },
        );
    }

    fn finish_terrain_region_column_request(
        &self,
        request_id: u64,
        result: Result<TerrainRegionColumnBatchResult, RemoteWorldError>,
    ) {
        let Some(pending) = self.pending.borrow_mut().remove(&request_id) else {
            return;
        };
        let PendingBatch::TerrainRegionColumns { columns, .. } = pending else {
            return;
        };
        self.terrain_region_column_completions
            .borrow_mut()
            .push_back(RemoteTerrainRegionColumnCompletion {
                request_id,
                columns,
                result,
            });
    }

    fn finish_terrain_page_request(
        &self,
        request_id: u64,
        result: Result<VirtualTerrainPageBatchResult, RemoteWorldError>,
    ) {
        let Some(pending) = self.pending.borrow_mut().remove(&request_id) else {
            return;
        };
        let PendingBatch::TerrainPages { requested, .. } = pending else {
            return;
        };
        self.terrain_page_completions
            .borrow_mut()
            .push_back(RemoteTerrainPageCompletion {
                request_id,
                requested,
                result,
            });
    }

    fn finish_pending_error(&self, request_id: u64, error: RemoteWorldError) {
        let kind = self
            .pending
            .borrow()
            .get(&request_id)
            .map(PendingBatch::kind);
        match kind {
            Some(PendingBatchKind::Chunks) => self.finish_request(request_id, Err(error)),
            Some(PendingBatchKind::TerrainRegionColumns) => {
                self.finish_terrain_region_column_request(request_id, Err(error))
            }
            Some(PendingBatchKind::TerrainDirectories) => {
                self.finish_terrain_directory_request(request_id, Err(error))
            }
            Some(PendingBatchKind::TerrainPages) => {
                self.finish_terrain_page_request(request_id, Err(error))
            }
            None => {}
        }
    }

    fn disconnect(self: &Rc<Self>, generation: u64, error: RemoteWorldError) {
        if generation != self.generation.get()
            || matches!(
                self.state.get(),
                RemoteConnectionState::WaitingToReconnect
                    | RemoteConnectionState::Failed
                    | RemoteConnectionState::Closed
            )
        {
            return;
        }
        self.detach_socket(1011, "world connection reset");
        self.opened.borrow_mut().take();
        self.send_paused.set(false);
        self.fail_pending(error.clone());

        let attempt = self.reconnect_attempts.get().saturating_add(1);
        if attempt > self.config.reconnect_attempt_limit {
            self.fail_terminal(error);
            return;
        }
        self.reconnect_attempts.set(attempt);
        self.state.set(RemoteConnectionState::WaitingToReconnect);
        let shift = attempt.saturating_sub(1).min(16);
        let delay = self
            .config
            .reconnect_initial_delay_ms
            .saturating_mul(1_u32 << shift)
            .min(self.config.reconnect_max_delay_ms);
        let weak = Rc::downgrade(self);
        let scheduled = schedule_after(delay, move || {
            let Some(inner) = weak.upgrade() else {
                return;
            };
            if inner.state.get() != RemoteConnectionState::WaitingToReconnect
                || inner.reconnect_attempts.get() != attempt
            {
                return;
            }
            if let Err(error) = RemoteInner::open_socket(&inner) {
                let generation = inner.generation.get();
                inner.state.set(RemoteConnectionState::Connecting);
                inner.disconnect(generation, error);
            }
        });
        if let Err(schedule_error) = scheduled {
            self.fail_terminal(schedule_error);
        }
    }

    fn fail_pending(&self, error: RemoteWorldError) {
        let request_ids = self.pending.borrow().keys().copied().collect::<Vec<_>>();
        for request_id in request_ids {
            self.finish_pending_error(request_id, error.clone());
        }
    }

    fn fail_terminal(&self, error: RemoteWorldError) {
        self.state.set(RemoteConnectionState::Failed);
        self.pending_edits.borrow_mut().clear();
        *self.terminal_error.borrow_mut() = Some(error.clone());
        for waiter in self.open_waiters.borrow_mut().drain(..) {
            waiter.send(Err(error.clone()));
        }
    }

    fn terminal_error(&self) -> Option<RemoteWorldError> {
        match self.state.get() {
            RemoteConnectionState::Failed | RemoteConnectionState::Closed => self
                .terminal_error
                .borrow()
                .clone()
                .or(Some(RemoteWorldError::Closed)),
            _ => None,
        }
    }

    fn detach_socket(&self, code: u16, reason: &str) {
        if let Some(socket) = self.socket.borrow_mut().take() {
            socket.set_onopen(None);
            socket.set_onmessage(None);
            socket.set_onerror(None);
            socket.set_onclose(None);
            let _ = socket.close_with_code_and_reason(code, reason);
        }
        self.handlers.borrow_mut().take();
        self.frame_reassembler.borrow_mut().clear();
    }

    fn close(&self) {
        if self.state.replace(RemoteConnectionState::Closed) == RemoteConnectionState::Closed {
            return;
        }
        self.generation.set(self.generation.get().wrapping_add(1));
        self.detach_socket(1000, "client closed");
        self.opened.borrow_mut().take();
        self.fail_pending(RemoteWorldError::Closed);
        self.pending_edits.borrow_mut().clear();
        self.edit_events.borrow_mut().clear();
        *self.terminal_error.borrow_mut() = Some(RemoteWorldError::Closed);
        for waiter in self.open_waiters.borrow_mut().drain(..) {
            waiter.send(Err(RemoteWorldError::Closed));
        }
    }
}

fn validate_chunk_result(
    result: &ChunkBatchResult,
    expected_coords: &[ChunkCoord],
    expected_identity: Option<WorldSourceIdentityHash>,
) -> Result<(), RemoteWorldError> {
    if Some(result.source_identity_hash) != expected_identity {
        return Err(RemoteWorldError::ResponseMismatch(
            "source identity changed",
        ));
    }
    if result.items.len() != expected_coords.len() {
        return Err(RemoteWorldError::ResponseMismatch(
            "item count differs from request",
        ));
    }
    let expected = expected_coords.iter().copied().collect::<BTreeSet<_>>();
    let returned = result
        .items
        .iter()
        .map(|item| item.coord)
        .collect::<BTreeSet<_>>();
    if expected.len() != expected_coords.len()
        || returned.len() != result.items.len()
        || returned != expected
    {
        return Err(RemoteWorldError::ResponseMismatch(
            "returned chunk keys differ from request",
        ));
    }
    Ok(())
}

fn validate_terrain_directory_result(
    result: &TerrainDirectoryBatchResult,
    expected_roots: &[voxels_world::TerrainPageKey],
    expected_identity: Option<WorldSourceIdentityHash>,
) -> Result<(), RemoteWorldError> {
    if Some(result.source_identity_hash) != expected_identity {
        return Err(RemoteWorldError::ResponseMismatch(
            "terrain directory source identity changed",
        ));
    }
    if result.items.len() != expected_roots.len()
        || !result
            .items
            .iter()
            .zip(expected_roots)
            .all(|(item, expected)| item.root == *expected)
    {
        return Err(RemoteWorldError::ResponseMismatch(
            "terrain directory roots differ from request",
        ));
    }
    Ok(())
}

fn validate_terrain_region_column_result(
    result: &TerrainRegionColumnBatchResult,
    expected_columns: &[[i32; 2]],
    expected_identity: Option<WorldSourceIdentityHash>,
) -> Result<(), RemoteWorldError> {
    if Some(result.source_identity_hash) != expected_identity {
        return Err(RemoteWorldError::ResponseMismatch(
            "terrain region column source identity changed",
        ));
    }
    if result.items.len() != expected_columns.len()
        || !result
            .items
            .iter()
            .zip(expected_columns)
            .all(|(item, expected)| item.column == *expected)
    {
        return Err(RemoteWorldError::ResponseMismatch(
            "terrain region columns differ from request",
        ));
    }
    Ok(())
}

fn validate_terrain_page_result(
    result: &VirtualTerrainPageBatchResult,
    expected_pages: &[TerrainPageTransferIdentity],
    expected_identity: WorldSourceIdentityHash,
) -> Result<(), RemoteWorldError> {
    if result.batch.source_identity_hash != expected_identity {
        return Err(RemoteWorldError::ResponseMismatch(
            "terrain page source identity changed",
        ));
    }
    if result.batch.items.len() != expected_pages.len()
        || !result
            .batch
            .items
            .iter()
            .zip(expected_pages)
            .all(|(item, expected)| item.requested == *expected)
    {
        return Err(RemoteWorldError::ResponseMismatch(
            "terrain page identities differ from request",
        ));
    }
    Ok(())
}

fn schedule_after(
    milliseconds: u32,
    callback: impl FnOnce() + 'static,
) -> Result<(), RemoteWorldError> {
    let scope: WorkerGlobalScope = js_sys::global().unchecked_into();
    let callback = Closure::once_into_js(callback);
    scope
        .set_timeout_with_callback_and_timeout_and_arguments_0(
            callback.unchecked_ref(),
            i32::try_from(milliseconds).unwrap_or(i32::MAX),
        )
        .map(|_| ())
        .map_err(|error| RemoteWorldError::Socket(js_reason(error)))
}

fn js_reason(value: wasm_bindgen::JsValue) -> String {
    value
        .as_string()
        .unwrap_or_else(|| format!("JavaScript exception at {:.0} ms", Date::now()))
}

struct LocalState<T> {
    value: Option<T>,
    waker: Option<Waker>,
}

struct LocalSender<T> {
    state: Rc<RefCell<LocalState<T>>>,
}

impl<T> LocalSender<T> {
    fn send(self, value: T) {
        let mut state = self.state.borrow_mut();
        state.value = Some(value);
        if let Some(waker) = state.waker.take() {
            waker.wake();
        }
    }
}

struct LocalReceiver<T> {
    state: Rc<RefCell<LocalState<T>>>,
}

impl<T> Future for LocalReceiver<T> {
    type Output = T;

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        let mut state = self.state.borrow_mut();
        if let Some(value) = state.value.take() {
            Poll::Ready(value)
        } else {
            state.waker = Some(context.waker().clone());
            Poll::Pending
        }
    }
}

fn local_channel<T>() -> (LocalSender<T>, LocalReceiver<T>) {
    let state = Rc::new(RefCell::new(LocalState {
        value: None,
        waker: None,
    }));
    (
        LocalSender {
            state: Rc::clone(&state),
        },
        LocalReceiver { state },
    )
}
