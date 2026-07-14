//! SQLite-on-OPFS persistence and multi-tab ownership. All database, filesystem, election, and
//! follower proxy work stays inside the Rust worker.
//!
//! The OPFS SAH pool is exclusive per origin. One tab therefore holds a Web Lock and owns the only
//! SQLite connection; other tabs proxy typed operations over a BroadcastChannel. Ownership moves to
//! a queued follower when the leader disappears. Installing the VFS retries because a predecessor's
//! sync access handle can remain busy briefly during a rapid reload or ownership handoff.

use rusqlite::{Connection, OptionalExtension, params};
use sqlite_wasm_vfs::sahpool::{OpfsSAHPoolCfg, OpfsSAHPoolUtil, install as install_opfs_sahpool};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::rc::Rc;
use voxels_core::CameraState;
use voxels_world::protocol::PlayerId;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use web_sys::{AbortController, BroadcastChannel, DedicatedWorkerGlobalScope, MessageEvent};

const SCHEMA_VERSION: i64 = 6;
const CHANNEL_NAME: &str = "voxels-db-p6";
const LOCK_NAME: &str = "voxels-db-p6-owner";
const WIRE_VERSION: f64 = 4.0;
const MSG_REQUEST: f64 = 0.0;
const MSG_RESPONSE: f64 = 1.0;
const RESULT_OK: f64 = 0.0;
const RESULT_ERROR: f64 = 1.0;
const OP_SAVE_CAMERA: f64 = 0.0;

type MessageHandler = Closure<dyn FnMut(MessageEvent)>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PersistenceConfig {
    pub request_timeout_ms: i32,
    pub request_retries: usize,
    pub vfs_install_attempts: usize,
    pub vfs_retry_delay_ms: i32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PersistencePlayer {
    pub player_id: PlayerId,
}

/// Complete namespace for one immutable world source.
///
/// The full manifest hash and current persistence schema define both the origin-wide routing tag and
/// database filename, so incompatible builds and world providers never share local state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PersistenceWorld {
    tag: String,
    database_name: String,
    database_seed: u64,
    database_version: u32,
}

impl PersistenceWorld {
    pub fn negotiated(manifest_hash: voxels_world::WorldManifestHash) -> Self {
        let bytes = manifest_hash.as_bytes();
        let mut seed_bytes = [0_u8; 8];
        seed_bytes.copy_from_slice(&bytes[..8]);
        let database_seed = u64::from_le_bytes(seed_bytes);
        let mut version_bytes = [0_u8; 4];
        version_bytes.copy_from_slice(&bytes[8..12]);
        let database_version = u32::from_le_bytes(version_bytes);
        let tag = format!("p{SCHEMA_VERSION}:manifest:{manifest_hash}");
        Self {
            database_name: format!("voxels-p{SCHEMA_VERSION}-{manifest_hash}.db"),
            tag,
            database_seed,
            database_version,
        }
    }
}

#[derive(Clone)]
enum Operation {
    SaveCamera {
        player_id: PlayerId,
        values: [f32; 5],
    },
}

enum OperationResult {
    Ok,
    Error(String),
}

struct Pending {
    resolve: js_sys::Function,
    slot: Rc<RefCell<Option<OperationResult>>>,
}

/// The leader owns both layers of the browser-local database. SQLite must close first, then the VFS
/// must pause to relinquish every synchronous OPFS access handle before another worker can install it.
struct Leader {
    connection: RefCell<Option<Connection>>,
    vfs: OpfsSAHPoolUtil,
    paused: Cell<bool>,
}

impl Leader {
    fn new(connection: Connection, vfs: OpfsSAHPoolUtil) -> Self {
        Self {
            connection: RefCell::new(Some(connection)),
            vfs,
            paused: Cell::new(false),
        }
    }

    fn run(&self, operation: &Operation) -> OperationResult {
        self.connection.borrow().as_ref().map_or_else(
            || OperationResult::Error("persistence leader is shutting down".into()),
            |connection| run_operation(connection, operation),
        )
    }

    fn shutdown(&self) -> bool {
        drop(self.connection.borrow_mut().take());
        if self.paused.get() {
            return true;
        }
        match self.vfs.pause_vfs() {
            Ok(()) => {
                self.paused.set(true);
                true
            }
            Err(error) => {
                web_sys::console::error_1(&JsValue::from_str(&format!(
                    "release OPFS SQLite VFS: {error}"
                )));
                false
            }
        }
    }
}

impl Drop for Leader {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

/// One origin-wide persistence coordinator. `leader` is populated exactly while this worker owns the
/// Web Lock. The lock callback's unresolved promise keeps ownership alive until the worker disappears.
struct Coordinator {
    config: PersistenceConfig,
    tab_id: f64,
    world_tag: String,
    channel: BroadcastChannel,
    leader: RefCell<Option<Rc<Leader>>>,
    leader_release: RefCell<Option<js_sys::Function>>,
    queued_lock_abort: RefCell<Option<AbortController>>,
    next_request: Cell<u64>,
    pending: RefCell<HashMap<u64, Pending>>,
    write_queue: RefCell<VecDeque<Operation>>,
    active_write: RefCell<Option<Operation>>,
    writing: Cell<bool>,
    closed: Cell<bool>,
    last_leader_error: RefCell<Option<String>>,
    _on_message: RefCell<Option<MessageHandler>>,
    world: PersistenceWorld,
    player: PersistencePlayer,
}

pub struct Store {
    coordinator: Rc<Coordinator>,
}

impl Store {
    pub async fn open(
        world: PersistenceWorld,
        player: PersistencePlayer,
        config: PersistenceConfig,
    ) -> Result<Self, JsValue> {
        Ok(Self {
            coordinator: Coordinator::start(world, player, config).await?,
        })
    }

    pub fn save_camera(&self, camera: &CameraState) -> Result<(), JsValue> {
        self.coordinator.dispatch_write(Operation::SaveCamera {
            player_id: self.coordinator.player.player_id,
            values: [
                camera.position.x,
                camera.position.y,
                camera.position.z,
                camera.yaw,
                camera.pitch,
            ],
        })
    }

    pub fn shutdown(&self) -> impl std::future::Future<Output = ()> + 'static {
        let coordinator = self.coordinator.clone();
        async move { coordinator.shutdown().await }
    }

    pub fn shutdown_now(&self) {
        self.coordinator.shutdown_now();
    }
}

impl Drop for Store {
    fn drop(&mut self) {
        self.shutdown_now();
    }
}

impl Coordinator {
    async fn start(
        world: PersistenceWorld,
        player: PersistencePlayer,
        config: PersistenceConfig,
    ) -> Result<Rc<Self>, JsValue> {
        let channel = BroadcastChannel::new(CHANNEL_NAME)?;
        let coordinator = Rc::new(Self {
            config,
            tab_id: js_sys::Math::random() * 9_007_199_254_740_991.0,
            world_tag: world.tag.clone(),
            channel,
            leader: RefCell::new(None),
            leader_release: RefCell::new(None),
            queued_lock_abort: RefCell::new(None),
            next_request: Cell::new(0),
            pending: RefCell::new(HashMap::new()),
            write_queue: RefCell::new(VecDeque::new()),
            active_write: RefCell::new(None),
            writing: Cell::new(false),
            closed: Cell::new(false),
            last_leader_error: RefCell::new(None),
            _on_message: RefCell::new(None),
            world,
            player,
        });
        coordinator.install_message_handler();
        coordinator.elect().await?;
        Ok(coordinator)
    }

    async fn request(self: &Rc<Self>, operation: Operation) -> OperationResult {
        for _ in 0..self.config.request_retries {
            if self.closed.get() {
                return OperationResult::Error("persistence coordinator is closed".into());
            }
            if let Some(leader) = self.leader.borrow().clone() {
                let result = leader.run(&operation);
                return result;
            }
            let id = self.next_request.get();
            self.next_request.set(id.wrapping_add(1));
            let slot = Rc::new(RefCell::new(None));
            let (promise, resolve) = resolvable();
            self.pending.borrow_mut().insert(
                id,
                Pending {
                    resolve,
                    slot: slot.clone(),
                },
            );
            self.post_request(id, &operation);
            let race = js_sys::Promise::race(&js_sys::Array::of2(
                &promise,
                &timeout(self.config.request_timeout_ms),
            ));
            let _ = JsFuture::from(race).await;
            self.pending.borrow_mut().remove(&id);
            let response = slot.borrow_mut().take();
            if let Some(response) = response {
                return response;
            }
        }
        OperationResult::Error(self.last_leader_error.borrow().clone().map_or_else(
            || "no persistence leader responded during ownership handoff".into(),
            |error| format!("persistence ownership recovery exhausted: {error}"),
        ))
    }

    fn dispatch_write(self: &Rc<Self>, operation: Operation) -> Result<(), JsValue> {
        if self.closed.get() {
            return Err(JsValue::from_str("persistence coordinator is closed"));
        }
        if let Some(leader) = self.leader.borrow().clone() {
            return operation_result(leader.run(&operation));
        }

        let mut queue = self.write_queue.borrow_mut();
        let Operation::SaveCamera { player_id, .. } = &operation;
        if let Some(pending) = queue.iter_mut().rev().find(|pending| {
            matches!(pending, Operation::SaveCamera { player_id: queued, .. } if queued == player_id)
        }) {
            *pending = operation;
        } else {
            queue.push_back(operation);
        }
        drop(queue);
        self.start_write_drain();
        Ok(())
    }

    fn start_write_drain(self: &Rc<Self>) {
        if self.writing.replace(true) {
            return;
        }
        let coordinator = self.clone();
        wasm_bindgen_futures::spawn_local(async move {
            while !coordinator.closed.get() {
                let Some(operation) = coordinator.write_queue.borrow_mut().pop_front() else {
                    break;
                };
                *coordinator.active_write.borrow_mut() = Some(operation.clone());
                let result = coordinator.request(operation).await;
                coordinator.active_write.borrow_mut().take();
                if let OperationResult::Error(error) = result {
                    web_sys::console::error_1(&JsValue::from_str(&format!(
                        "persistence follower write failed: {error}"
                    )));
                }
            }
            coordinator.writing.set(false);
            if !coordinator.closed.get() && !coordinator.write_queue.borrow().is_empty() {
                coordinator.start_write_drain();
            }
        });
    }

    fn install_message_handler(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        let handler = Closure::wrap(Box::new(move |event: MessageEvent| {
            let Some(coordinator) = weak.upgrade() else {
                return;
            };
            let Ok(message) = event.data().dyn_into::<js_sys::Array>() else {
                return;
            };
            coordinator.handle_message(&message);
        }) as Box<dyn FnMut(MessageEvent)>);
        self.channel
            .set_onmessage(Some(handler.as_ref().unchecked_ref()));
        *self._on_message.borrow_mut() = Some(handler);
    }

    fn handle_message(self: &Rc<Self>, message: &js_sys::Array) {
        if self.closed.get() {
            return;
        }
        if number(message, 1) != Some(WIRE_VERSION) {
            return;
        }
        match number(message, 0) {
            Some(MSG_REQUEST) => self.handle_request(message),
            Some(MSG_RESPONSE) => self.handle_response(message),
            _ => {}
        }
    }

    fn handle_request(self: &Rc<Self>, message: &js_sys::Array) {
        let (Some(id), Some(from), Some(world_tag)) = (
            integer(message, 2),
            number(message, 3),
            message.get(4).as_string(),
        ) else {
            return;
        };
        let Some(leader) = self.leader.borrow().clone() else {
            return;
        };
        let result = if world_tag == self.world_tag {
            match decode_operation(message) {
                Ok(operation) => leader.run(&operation),
                Err(error) => OperationResult::Error(error),
            }
        } else {
            OperationResult::Error("persistence leader belongs to a different world build".into())
        };
        self.post_response(id, from, &result);
    }

    fn handle_response(&self, message: &js_sys::Array) {
        let (Some(id), Some(to)) = (integer(message, 2), number(message, 3)) else {
            return;
        };
        if to != self.tab_id {
            return;
        }
        let Some(result) = decode_result(message) else {
            return;
        };
        if let Some(pending) = self.pending.borrow_mut().remove(&id) {
            *pending.slot.borrow_mut() = Some(result);
            let _ = pending.resolve.call0(&JsValue::NULL);
        }
    }

    fn post_request(&self, id: u64, operation: &Operation) {
        let message = js_sys::Array::new();
        push_number(&message, MSG_REQUEST);
        push_number(&message, WIRE_VERSION);
        push_number(&message, id as f64);
        push_number(&message, self.tab_id);
        message.push(&JsValue::from_str(&self.world_tag));
        encode_operation(&message, operation);
        let _ = self.channel.post_message(&message);
    }

    fn post_response(&self, id: u64, to: f64, result: &OperationResult) {
        let message = js_sys::Array::new();
        push_number(&message, MSG_RESPONSE);
        push_number(&message, WIRE_VERSION);
        push_number(&message, id as f64);
        push_number(&message, to);
        encode_result(&message, result);
        let _ = self.channel.post_message(&message);
    }

    async fn elect(self: &Rc<Self>) -> Result<(), JsValue> {
        let (ready, ready_resolve) = resolvable();
        let coordinator = self.clone();
        let probe = Closure::once_into_js(move |lock: JsValue| -> js_sys::Promise {
            if lock.is_null() {
                let _ = ready_resolve.call0(&JsValue::NULL);
                js_sys::Promise::resolve(&JsValue::NULL)
            } else {
                coordinator.become_leader(Some(ready_resolve))
            }
        });
        let options = js_sys::Object::new();
        js_sys::Reflect::set(&options, &JsValue::from_str("ifAvailable"), &JsValue::TRUE)?;
        request_lock(&[&JsValue::from_str(LOCK_NAME), &options, &probe])?;
        JsFuture::from(ready).await?;
        if self.leader.borrow().is_none() && !self.closed.get() {
            self.queue_for_handoff();
        }
        Ok(())
    }

    fn become_leader(self: &Rc<Self>, ready: Option<js_sys::Function>) -> js_sys::Promise {
        let (hold, release) = resolvable();
        let boot_probe = ready.is_some();
        let coordinator = self.clone();
        wasm_bindgen_futures::spawn_local(async move {
            match open_leader(coordinator.world.clone(), coordinator.config).await {
                Ok(leader) if !coordinator.closed.get() => {
                    coordinator.last_leader_error.borrow_mut().take();
                    *coordinator.leader.borrow_mut() = Some(Rc::new(leader));
                    *coordinator.leader_release.borrow_mut() = Some(release);
                    if let Some(ready) = ready {
                        let _ = ready.call0(&JsValue::NULL);
                    }
                }
                Ok(leader) => {
                    let released = leader.shutdown();
                    if let Some(ready) = ready {
                        let _ = ready.call0(&JsValue::NULL);
                    }
                    if released {
                        let _ = release.call0(&JsValue::NULL);
                    }
                }
                Err(error) => {
                    // A predecessor can release its Web Lock a little before the browser has finished
                    // releasing every synchronous OPFS handle. That is a recoverable ownership
                    // transition, not an engine error. Retain the reason so Store::open can report it
                    // once, as a fatal error, only if all election/request retries are exhausted.
                    *coordinator.last_leader_error.borrow_mut() = Some(js_value_message(&error));
                    if let Some(ready) = ready {
                        let _ = ready.call0(&JsValue::NULL);
                    }
                    if !boot_probe && !coordinator.closed.get() {
                        coordinator.queue_for_handoff();
                    }
                    let _ = release.call0(&JsValue::NULL);
                }
            }
        });
        hold
    }

    fn queue_for_handoff(self: &Rc<Self>) {
        if self.closed.get() || self.queued_lock_abort.borrow().is_some() {
            return;
        }
        let Ok(abort) = AbortController::new() else {
            return;
        };
        let weak = Rc::downgrade(self);
        let callback = Closure::once_into_js(move |_lock: JsValue| -> js_sys::Promise {
            let Some(coordinator) = weak.upgrade() else {
                return js_sys::Promise::resolve(&JsValue::NULL);
            };
            coordinator.queued_lock_abort.borrow_mut().take();
            if coordinator.closed.get() {
                js_sys::Promise::resolve(&JsValue::NULL)
            } else {
                coordinator.become_leader(None)
            }
        });
        let options = js_sys::Object::new();
        if js_sys::Reflect::set(
            &options,
            &JsValue::from_str("signal"),
            abort.signal().as_ref(),
        )
        .is_err()
        {
            return;
        }
        if request_lock(&[&JsValue::from_str(LOCK_NAME), &options, &callback]).is_ok() {
            *self.queued_lock_abort.borrow_mut() = Some(abort);
        }
    }

    async fn shutdown(self: &Rc<Self>) {
        if self.closed.get() {
            return;
        }
        self.flush_queued_writes();
        for _ in 0..40 {
            if !self.writing.get() && self.write_queue.borrow().is_empty() {
                break;
            }
            let _ = JsFuture::from(timeout(10)).await;
        }
        self.shutdown_now();
    }

    fn shutdown_now(&self) {
        if self.closed.get() {
            return;
        }
        self.flush_queued_writes();
        if self.closed.replace(true) {
            return;
        }
        if let Some(abort) = self.queued_lock_abort.borrow_mut().take() {
            abort.abort();
        }
        let opfs_released = self
            .leader
            .borrow_mut()
            .take()
            .is_none_or(|leader| leader.shutdown());
        if opfs_released && let Some(release) = self.leader_release.borrow_mut().take() {
            let _ = release.call0(&JsValue::NULL);
        }
        for (_, pending) in self.pending.borrow_mut().drain() {
            *pending.slot.borrow_mut() = Some(OperationResult::Error(
                "persistence coordinator closed during request".into(),
            ));
            let _ = pending.resolve.call0(&JsValue::NULL);
        }
        self.channel.set_onmessage(None);
        self._on_message.borrow_mut().take();
        self.channel.close();
    }

    /// Hands every unfinished write to the current owner before this worker closes. This includes
    /// the operation removed from the queue by the async follower drain but still awaiting its
    /// response. BroadcastChannel clones a posted message synchronously, so the leader can finish
    /// the write after the follower has released its own channel and worker.
    fn flush_queued_writes(&self) {
        let active = self.active_write.borrow_mut().take();
        let queued = std::mem::take(&mut *self.write_queue.borrow_mut());
        let unfinished = active.into_iter().chain(queued);
        if let Some(leader) = self.leader.borrow().clone() {
            for operation in unfinished {
                let result = leader.run(&operation);
                if let OperationResult::Error(error) = result {
                    web_sys::console::error_1(&JsValue::from_str(&format!(
                        "flush persistence write during shutdown: {error}"
                    )));
                }
            }
            return;
        }
        for operation in unfinished {
            let id = self.next_request.get();
            self.next_request.set(id.wrapping_add(1));
            self.post_request(id, &operation);
        }
    }
}

async fn install_vfs(config: PersistenceConfig) -> Result<OpfsSAHPoolUtil, JsValue> {
    let mut last_error = String::new();
    for attempt in 0..config.vfs_install_attempts {
        match install_opfs_sahpool::<sqlite_wasm_rs::WasmOsCallback>(
            &OpfsSAHPoolCfg::default(),
            true,
        )
        .await
        {
            Ok(vfs) => return Ok(vfs),
            Err(error) => {
                last_error = error.to_string();
                if attempt + 1 < config.vfs_install_attempts {
                    JsFuture::from(timeout(config.vfs_retry_delay_ms)).await?;
                }
            }
        }
    }
    Err(JsValue::from_str(&format!(
        "install OPFS SQLite VFS failed after {} attempts: {last_error}",
        config.vfs_install_attempts
    )))
}

async fn open_leader(
    world: PersistenceWorld,
    config: PersistenceConfig,
) -> Result<Leader, JsValue> {
    let vfs = install_vfs(config).await?;
    let connection = (|| {
        let connection = Connection::open(&world.database_name)
            .map_err(|error| js_error("open SQLite database", error))?;
        connection
            .execute_batch("PRAGMA synchronous=NORMAL; PRAGMA foreign_keys=ON;")
            .map_err(|error| js_error("configure SQLite", error))?;
        initialize_current_schema(&connection)?;
        ensure_world(&connection, world.database_seed, world.database_version)?;
        Ok(connection)
    })();
    match connection {
        Ok(connection) => Ok(Leader::new(connection, vfs)),
        Err(error) => {
            if let Err(pause_error) = vfs.pause_vfs() {
                web_sys::console::error_1(&JsValue::from_str(&format!(
                    "release OPFS VFS after failed open: {pause_error}"
                )));
            }
            Err(error)
        }
    }
}

fn run_operation(connection: &Connection, operation: &Operation) -> OperationResult {
    match operation {
        Operation::SaveCamera { player_id, values } => save_camera(connection, *player_id, *values),
    }
}

fn save_camera(connection: &Connection, player_id: PlayerId, values: [f32; 5]) -> OperationResult {
    connection
        .execute(
            "INSERT INTO local_player_state (player_id, x, y, z, yaw, pitch) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
             ON CONFLICT(player_id) DO UPDATE SET x=excluded.x, y=excluded.y, z=excluded.z, \
             yaw=excluded.yaw, pitch=excluded.pitch, updated_at=unixepoch()",
            params![
                player_id.as_bytes().as_slice(),
                values[0],
                values[1],
                values[2],
                values[3],
                values[4]
            ],
        )
        .map(|_| OperationResult::Ok)
        .unwrap_or_else(|error| OperationResult::Error(format!("save camera: {error}")))
}

fn encode_operation(message: &js_sys::Array, operation: &Operation) {
    match operation {
        Operation::SaveCamera { player_id, values } => {
            push_number(message, OP_SAVE_CAMERA);
            message.push(&JsValue::from_str(&player_id.to_string()));
            for value in values {
                push_number(message, f64::from(*value));
            }
        }
    }
}

fn decode_operation(message: &js_sys::Array) -> Result<Operation, String> {
    match number(message, 5) {
        Some(OP_SAVE_CAMERA) => Ok(Operation::SaveCamera {
            player_id: decode_player_id(message, 6)?,
            values: [
                finite_f32(message, 7)?,
                finite_f32(message, 8)?,
                finite_f32(message, 9)?,
                finite_f32(message, 10)?,
                finite_f32(message, 11)?,
            ],
        }),
        _ => Err("unknown persistence operation".into()),
    }
}

fn decode_player_id(message: &js_sys::Array, index: u32) -> Result<PlayerId, String> {
    let value = message
        .get(index)
        .as_string()
        .ok_or_else(|| "missing player id in persistence message".to_owned())?;
    let player_id = PlayerId::from_uuid_str(&value)
        .ok_or_else(|| "invalid player id in persistence message".to_owned())?;
    if player_id.is_nil() {
        return Err("nil player id in persistence message".to_owned());
    }
    Ok(player_id)
}

fn encode_result(message: &js_sys::Array, result: &OperationResult) {
    match result {
        OperationResult::Ok => push_number(message, RESULT_OK),
        OperationResult::Error(error) => {
            push_number(message, RESULT_ERROR);
            message.push(&JsValue::from_str(error));
        }
    }
}

fn decode_result(message: &js_sys::Array) -> Option<OperationResult> {
    match number(message, 4)? {
        RESULT_OK => Some(OperationResult::Ok),
        RESULT_ERROR => Some(OperationResult::Error(message.get(5).as_string()?)),
        _ => None,
    }
}

fn operation_result(result: OperationResult) -> Result<(), JsValue> {
    match result {
        OperationResult::Ok => Ok(()),
        OperationResult::Error(error) => Err(JsValue::from_str(&error)),
    }
}

fn number(message: &js_sys::Array, index: u32) -> Option<f64> {
    message.get(index).as_f64()
}

fn integer(message: &js_sys::Array, index: u32) -> Option<u64> {
    let value = number(message, index)?;
    (value.is_finite() && value >= 0.0 && value.fract() == 0.0 && value <= 9_007_199_254_740_991.0)
        .then_some(value as u64)
}

fn finite_number(message: &js_sys::Array, index: u32) -> Result<f64, String> {
    let value = number(message, index).ok_or_else(|| "missing numeric wire value".to_string())?;
    if value.is_finite() {
        Ok(value)
    } else {
        Err("non-finite numeric wire value".into())
    }
}

fn finite_f32(message: &js_sys::Array, index: u32) -> Result<f32, String> {
    let value = finite_number(message, index)?;
    if value >= f64::from(f32::MIN) && value <= f64::from(f32::MAX) {
        Ok(value as f32)
    } else {
        Err("numeric wire value is outside f32 range".into())
    }
}

fn push_number(message: &js_sys::Array, value: f64) {
    message.push(&JsValue::from_f64(value));
}

fn request_lock(arguments: &[&JsValue]) -> Result<(), JsValue> {
    let navigator: JsValue = worker_scope().navigator().into();
    let locks = js_sys::Reflect::get(&navigator, &JsValue::from_str("locks"))?;
    let request: js_sys::Function =
        js_sys::Reflect::get(&locks, &JsValue::from_str("request"))?.unchecked_into();
    let args = js_sys::Array::new();
    for argument in arguments {
        args.push(argument);
    }
    let promise: js_sys::Promise = js_sys::Reflect::apply(&request, &locks, &args)?.dyn_into()?;
    let on_rejected = Closure::once_into_js(move |error: JsValue| -> JsValue {
        let name = js_sys::Reflect::get(&error, &JsValue::from_str("name"))
            .ok()
            .and_then(|name| name.as_string());
        if name.as_deref() != Some("AbortError") {
            web_sys::console::error_1(&JsValue::from_str(&format!(
                "Web Lock request failed: {}",
                js_value_message(&error)
            )));
        }
        JsValue::UNDEFINED
    });
    let catch: js_sys::Function =
        js_sys::Reflect::get(&promise, &JsValue::from_str("catch"))?.unchecked_into();
    js_sys::Reflect::apply(&catch, &promise, &js_sys::Array::of1(&on_rejected))?;
    Ok(())
}

fn worker_scope() -> DedicatedWorkerGlobalScope {
    js_sys::global().unchecked_into()
}

fn timeout(milliseconds: i32) -> js_sys::Promise {
    js_sys::Promise::new(&mut |resolve, _reject| {
        let _ = worker_scope()
            .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, milliseconds);
    })
}

fn resolvable() -> (js_sys::Promise, js_sys::Function) {
    let slot = Rc::new(RefCell::new(None));
    let captured = slot.clone();
    let promise = js_sys::Promise::new(&mut |resolve, _reject| {
        *captured.borrow_mut() = Some(resolve);
    });
    #[allow(
        clippy::expect_used,
        reason = "the JavaScript Promise constructor invokes its executor synchronously by contract"
    )]
    let resolve = slot
        .borrow_mut()
        .take()
        .expect("Promise executor runs synchronously");
    (promise, resolve)
}

fn js_value_message(value: &JsValue) -> String {
    value.as_string().unwrap_or_else(|| format!("{value:?}"))
}

fn initialize_current_schema(connection: &Connection) -> Result<(), JsValue> {
    let version: i64 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(|error| js_error("read schema version", error))?;
    if version != 0 && version != SCHEMA_VERSION {
        return Err(JsValue::from_str(&format!(
            "local database schema {version} is unsupported; this build reads only schema {SCHEMA_VERSION}"
        )));
    }
    if version == 0 {
        connection
            .execute_batch(
                "BEGIN IMMEDIATE;
                 CREATE TABLE worlds (
                   id INTEGER PRIMARY KEY CHECK (id = 0),
                   seed BLOB NOT NULL CHECK (length(seed) = 8),
                   generator_version INTEGER NOT NULL,
                   created_at INTEGER NOT NULL DEFAULT (unixepoch())
                 );
                 CREATE TABLE local_player_state (
                   player_id BLOB PRIMARY KEY CHECK (length(player_id) = 16),
                   x REAL NOT NULL,
                   y REAL NOT NULL,
                   z REAL NOT NULL,
                   yaw REAL NOT NULL,
                   pitch REAL NOT NULL,
                   updated_at INTEGER NOT NULL DEFAULT (unixepoch())
                 ) WITHOUT ROWID;
                 PRAGMA user_version=6;
                 COMMIT;",
            )
            .map_err(|error| js_error("initialize local database schema", error))?;
    }
    Ok(())
}

fn ensure_world(
    connection: &Connection,
    world_seed: u64,
    generator_version: u32,
) -> Result<(), JsValue> {
    let stored: Option<(Vec<u8>, u32)> = connection
        .query_row(
            "SELECT seed, generator_version FROM worlds WHERE id = 0",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|error| js_error("load world identity", error))?;
    if let Some((seed, version)) = stored {
        if seed.as_slice() != world_seed.to_le_bytes() || version != generator_version {
            return Err(JsValue::from_str(
                "saved world identity does not match this generator build",
            ));
        }
    } else {
        connection
            .execute(
                "INSERT INTO worlds (id, seed, generator_version) VALUES (0, ?1, ?2)",
                params![world_seed.to_le_bytes().as_slice(), generator_version],
            )
            .map_err(|error| js_error("create world identity", error))?;
    }
    Ok(())
}

fn js_error(context: &str, error: impl std::fmt::Display) -> JsValue {
    JsValue::from_str(&format!("{context}: {error}"))
}
