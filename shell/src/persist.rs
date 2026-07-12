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
use voxels_world::{EditMap, Material, VoxelCoord};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use web_sys::{AbortController, BroadcastChannel, DedicatedWorkerGlobalScope, MessageEvent};

const DATABASE_NAME: &str = "voxels.db";
const SCHEMA_VERSION: i64 = 3;
const CHANNEL_NAME: &str = "voxels-db-v1";
const LOCK_NAME: &str = "voxels-db-owner";
const WIRE_VERSION: f64 = 1.0;
const REQUEST_TIMEOUT_MS: i32 = 400;
const REQUEST_RETRIES: usize = 30;
const VFS_INSTALL_ATTEMPTS: usize = 20;
const VFS_RETRY_DELAY_MS: i32 = 150;

const MSG_REQUEST: f64 = 0.0;
const MSG_RESPONSE: f64 = 1.0;
const MSG_EDIT_COMMITTED: f64 = 2.0;
const RESULT_OK: f64 = 0.0;
const RESULT_ERROR: f64 = 1.0;
const RESULT_NO_CAMERA: f64 = 2.0;
const RESULT_CAMERA: f64 = 3.0;
const RESULT_EDITS: f64 = 4.0;
const OP_LOAD_CAMERA: f64 = 0.0;
const OP_LOAD_EDITS: f64 = 1.0;
const OP_SAVE_CAMERA: f64 = 2.0;
const OP_SAVE_EDIT: f64 = 3.0;

type MessageHandler = Closure<dyn FnMut(MessageEvent)>;

#[derive(Clone)]
enum Operation {
    LoadCamera,
    LoadEdits,
    SaveCamera([f32; 5]),
    SaveEdit {
        coord: VoxelCoord,
        material: Option<u16>,
    },
}

enum OperationResult {
    Ok,
    Camera(Option<[f32; 5]>),
    Edits(Vec<(VoxelCoord, u16)>),
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
    tab_id: f64,
    world_tag: String,
    channel: BroadcastChannel,
    leader: RefCell<Option<Rc<Leader>>>,
    leader_release: RefCell<Option<js_sys::Function>>,
    queued_lock_abort: RefCell<Option<AbortController>>,
    next_request: Cell<u64>,
    pending: RefCell<HashMap<u64, Pending>>,
    write_queue: RefCell<VecDeque<Operation>>,
    committed_edits: RefCell<VecDeque<(VoxelCoord, Option<u16>)>>,
    writing: Cell<bool>,
    closed: Cell<bool>,
    last_leader_error: RefCell<Option<String>>,
    _on_message: RefCell<Option<MessageHandler>>,
    world_seed: u64,
    generator_version: u32,
}

pub struct Store {
    coordinator: Rc<Coordinator>,
    initial_camera: Option<CameraState>,
    initial_edits: Option<EditMap>,
}

impl Store {
    pub async fn open(world_seed: u64, generator_version: u32) -> Result<Self, JsValue> {
        let coordinator = Coordinator::start(world_seed, generator_version).await?;
        let initial_camera = match coordinator.request(Operation::LoadCamera).await {
            OperationResult::Camera(camera) => camera.map(camera_from_values),
            OperationResult::Error(error) => return Err(JsValue::from_str(&error)),
            _ => {
                return Err(JsValue::from_str(
                    "camera load returned an invalid response",
                ));
            }
        };
        let initial_edits = match coordinator.request(Operation::LoadEdits).await {
            OperationResult::Edits(rows) => edit_map_from_rows(rows)?,
            OperationResult::Error(error) => return Err(JsValue::from_str(&error)),
            _ => return Err(JsValue::from_str("edit load returned an invalid response")),
        };
        Ok(Self {
            coordinator,
            initial_camera,
            initial_edits: Some(initial_edits),
        })
    }

    pub fn load_camera(&mut self) -> Result<Option<CameraState>, JsValue> {
        Ok(self.initial_camera.take())
    }

    pub fn save_camera(&self, camera: &CameraState) -> Result<(), JsValue> {
        self.coordinator.dispatch_write(Operation::SaveCamera([
            camera.position.x,
            camera.position.y,
            camera.position.z,
            camera.yaw,
            camera.pitch,
        ]))
    }

    pub fn load_edits(&mut self) -> Result<EditMap, JsValue> {
        self.initial_edits
            .take()
            .ok_or_else(|| JsValue::from_str("initial edits were already loaded"))
    }

    pub fn save_edit(&self, coord: VoxelCoord, material: Option<Material>) -> Result<(), JsValue> {
        self.coordinator.dispatch_write(Operation::SaveEdit {
            coord,
            material: material.map(Material::id),
        })
    }

    pub fn owns_persistence(&self) -> bool {
        self.coordinator.leader.borrow().is_some()
    }

    pub fn shutdown(&self) -> impl std::future::Future<Output = ()> + 'static {
        let coordinator = self.coordinator.clone();
        async move { coordinator.shutdown().await }
    }

    pub fn shutdown_now(&self) {
        self.coordinator.shutdown_now();
    }

    pub fn drain_remote_edits(&self) -> Result<Vec<(VoxelCoord, Option<Material>)>, JsValue> {
        self.coordinator
            .committed_edits
            .borrow_mut()
            .drain(..)
            .map(|(coord, material)| {
                material.map_or(Ok((coord, None)), |id| {
                    Material::from_id(id)
                        .map(|material| (coord, Some(material)))
                        .ok_or_else(|| {
                            JsValue::from_str(&format!("unknown committed material {id}"))
                        })
                })
            })
            .collect()
    }
}

impl Drop for Store {
    fn drop(&mut self) {
        self.shutdown_now();
    }
}

impl Coordinator {
    async fn start(world_seed: u64, generator_version: u32) -> Result<Rc<Self>, JsValue> {
        let channel = BroadcastChannel::new(CHANNEL_NAME)?;
        let coordinator = Rc::new(Self {
            tab_id: js_sys::Math::random() * 9_007_199_254_740_991.0,
            world_tag: format!("{world_seed:016x}:{generator_version}"),
            channel,
            leader: RefCell::new(None),
            leader_release: RefCell::new(None),
            queued_lock_abort: RefCell::new(None),
            next_request: Cell::new(0),
            pending: RefCell::new(HashMap::new()),
            write_queue: RefCell::new(VecDeque::new()),
            committed_edits: RefCell::new(VecDeque::new()),
            writing: Cell::new(false),
            closed: Cell::new(false),
            last_leader_error: RefCell::new(None),
            _on_message: RefCell::new(None),
            world_seed,
            generator_version,
        });
        coordinator.install_message_handler();
        coordinator.elect().await?;
        Ok(coordinator)
    }

    async fn request(self: &Rc<Self>, operation: Operation) -> OperationResult {
        for _ in 0..REQUEST_RETRIES {
            if self.closed.get() {
                return OperationResult::Error("persistence coordinator is closed".into());
            }
            if let Some(leader) = self.leader.borrow().clone() {
                let result = leader.run(&operation);
                if matches!(result, OperationResult::Ok)
                    && let Operation::SaveEdit { coord, material } = operation
                {
                    self.post_edit_committed(self.tab_id, coord, material);
                }
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
            let race =
                js_sys::Promise::race(&js_sys::Array::of2(&promise, &timeout(REQUEST_TIMEOUT_MS)));
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
            let result = leader.run(&operation);
            if matches!(result, OperationResult::Ok)
                && let Operation::SaveEdit { coord, material } = operation
            {
                self.post_edit_committed(self.tab_id, coord, material);
            }
            return operation_result(result);
        }

        let mut queue = self.write_queue.borrow_mut();
        match &operation {
            Operation::SaveCamera(_) => {
                if let Some(pending) = queue
                    .iter_mut()
                    .rev()
                    .find(|pending| matches!(pending, Operation::SaveCamera(_)))
                {
                    *pending = operation;
                } else {
                    queue.push_back(operation);
                }
            }
            Operation::SaveEdit { coord, .. } => {
                if let Some(pending) = queue.iter_mut().rev().find(|pending| {
                    matches!(pending, Operation::SaveEdit { coord: queued, .. } if queued == coord)
                }) {
                    *pending = operation;
                } else {
                    queue.push_back(operation);
                }
            }
            _ => queue.push_back(operation),
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
                if let OperationResult::Error(error) = coordinator.request(operation).await {
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
            Some(MSG_EDIT_COMMITTED) => self.handle_edit_committed(message),
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
                Ok(operation) => {
                    let result = leader.run(&operation);
                    if matches!(result, OperationResult::Ok)
                        && let Operation::SaveEdit { coord, material } = operation
                    {
                        // BroadcastChannel does not echo to this leader's channel object, so apply a
                        // follower-originated commit to the leader engine explicitly as well.
                        self.committed_edits
                            .borrow_mut()
                            .push_back((coord, material));
                        self.post_edit_committed(from, coord, material);
                    }
                    result
                }
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

    fn handle_edit_committed(&self, message: &js_sys::Array) {
        let (Some(_origin), Some(world_tag)) = (number(message, 2), message.get(3).as_string())
        else {
            return;
        };
        if world_tag != self.world_tag {
            return;
        }
        let Ok(coord) = decode_coord(message, 4) else {
            return;
        };
        let Ok(material) = decode_optional_material(message, 7) else {
            return;
        };
        self.committed_edits
            .borrow_mut()
            .push_back((coord, material));
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

    fn post_edit_committed(&self, origin: f64, coord: VoxelCoord, material: Option<u16>) {
        let message = js_sys::Array::new();
        push_number(&message, MSG_EDIT_COMMITTED);
        push_number(&message, WIRE_VERSION);
        push_number(&message, origin);
        message.push(&JsValue::from_str(&self.world_tag));
        push_number(&message, f64::from(coord.x));
        push_number(&message, f64::from(coord.y));
        push_number(&message, f64::from(coord.z));
        push_number(&message, material.map_or(-1.0, f64::from));
        if let Err(error) = self.channel.post_message(&message) {
            web_sys::console::error_1(&JsValue::from_str(&format!(
                "broadcast committed voxel edit: {}",
                js_value_message(&error)
            )));
        }
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
            match open_leader(coordinator.world_seed, coordinator.generator_version).await {
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
        self.committed_edits.borrow_mut().clear();
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

    /// Hands writes that have not entered the async follower drain to the current owner before this
    /// worker closes. BroadcastChannel clones a posted message synchronously, so the leader can
    /// finish the write after the follower has released its own channel and worker.
    fn flush_queued_writes(&self) {
        let queued = std::mem::take(&mut *self.write_queue.borrow_mut());
        if let Some(leader) = self.leader.borrow().clone() {
            for operation in queued {
                let result = leader.run(&operation);
                if matches!(result, OperationResult::Ok)
                    && let Operation::SaveEdit { coord, material } = operation
                {
                    self.post_edit_committed(self.tab_id, coord, material);
                }
                if let OperationResult::Error(error) = result {
                    web_sys::console::error_1(&JsValue::from_str(&format!(
                        "flush persistence write during shutdown: {error}"
                    )));
                }
            }
            return;
        }
        for operation in queued {
            let id = self.next_request.get();
            self.next_request.set(id.wrapping_add(1));
            self.post_request(id, &operation);
        }
    }
}

async fn install_vfs() -> Result<OpfsSAHPoolUtil, JsValue> {
    let mut last_error = String::new();
    for attempt in 0..VFS_INSTALL_ATTEMPTS {
        match install_opfs_sahpool::<sqlite_wasm_rs::WasmOsCallback>(
            &OpfsSAHPoolCfg::default(),
            true,
        )
        .await
        {
            Ok(vfs) => return Ok(vfs),
            Err(error) => {
                last_error = error.to_string();
                if attempt + 1 < VFS_INSTALL_ATTEMPTS {
                    JsFuture::from(timeout(VFS_RETRY_DELAY_MS)).await?;
                }
            }
        }
    }
    Err(JsValue::from_str(&format!(
        "install OPFS SQLite VFS failed after {VFS_INSTALL_ATTEMPTS} attempts: {last_error}"
    )))
}

async fn open_leader(world_seed: u64, generator_version: u32) -> Result<Leader, JsValue> {
    let vfs = install_vfs().await?;
    let connection = (|| {
        let connection = Connection::open(DATABASE_NAME)
            .map_err(|error| js_error("open SQLite database", error))?;
        connection
            .execute_batch("PRAGMA synchronous=NORMAL; PRAGMA foreign_keys=ON;")
            .map_err(|error| js_error("configure SQLite", error))?;
        migrate(&connection)?;
        ensure_world(&connection, world_seed, generator_version)?;
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
        Operation::LoadCamera => load_camera(connection),
        Operation::LoadEdits => load_edits(connection),
        Operation::SaveCamera(values) => save_camera(connection, *values),
        Operation::SaveEdit { coord, material } => save_edit(connection, *coord, *material),
    }
}

fn load_camera(connection: &Connection) -> OperationResult {
    connection
        .query_row(
            "SELECT x, y, z, yaw, pitch FROM camera WHERE id = 0",
            [],
            |row| {
                Ok([
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ])
            },
        )
        .optional()
        .map(OperationResult::Camera)
        .unwrap_or_else(|error| OperationResult::Error(format!("load camera: {error}")))
}

fn save_camera(connection: &Connection, values: [f32; 5]) -> OperationResult {
    connection
        .execute(
            "INSERT INTO camera (id, x, y, z, yaw, pitch) VALUES (0, ?1, ?2, ?3, ?4, ?5) \
             ON CONFLICT(id) DO UPDATE SET x=excluded.x, y=excluded.y, z=excluded.z, \
             yaw=excluded.yaw, pitch=excluded.pitch",
            params![values[0], values[1], values[2], values[3], values[4]],
        )
        .map(|_| OperationResult::Ok)
        .unwrap_or_else(|error| OperationResult::Error(format!("save camera: {error}")))
}

fn load_edits(connection: &Connection) -> OperationResult {
    let mut statement =
        match connection.prepare("SELECT x, y, z, material FROM voxel_edits WHERE world_id = 0") {
            Ok(statement) => statement,
            Err(error) => return OperationResult::Error(format!("prepare edit load: {error}")),
        };
    let rows = match statement.query_map([], |row| {
        Ok((
            VoxelCoord::new(row.get(0)?, row.get(1)?, row.get(2)?),
            row.get::<_, u16>(3)?,
        ))
    }) {
        Ok(rows) => rows,
        Err(error) => return OperationResult::Error(format!("load edit rows: {error}")),
    };
    let mut edits = Vec::new();
    for row in rows {
        match row {
            Ok(row) => edits.push(row),
            Err(error) => return OperationResult::Error(format!("decode edit row: {error}")),
        }
    }
    OperationResult::Edits(edits)
}

fn save_edit(connection: &Connection, coord: VoxelCoord, material: Option<u16>) -> OperationResult {
    let result = if let Some(material) = material {
        connection.execute(
            "INSERT INTO voxel_edits (world_id, x, y, z, material) VALUES (0, ?1, ?2, ?3, ?4) \
             ON CONFLICT(world_id, x, y, z) DO UPDATE SET material=excluded.material, \
             updated_at=unixepoch()",
            params![coord.x, coord.y, coord.z, material],
        )
    } else {
        connection.execute(
            "DELETE FROM voxel_edits WHERE world_id=0 AND x=?1 AND y=?2 AND z=?3",
            params![coord.x, coord.y, coord.z],
        )
    };
    result
        .map(|_| OperationResult::Ok)
        .unwrap_or_else(|error| OperationResult::Error(format!("persist voxel edit: {error}")))
}

fn camera_from_values(values: [f32; 5]) -> CameraState {
    CameraState::from_persisted(
        glam::Vec3::new(values[0], values[1], values[2]),
        values[3],
        values[4],
    )
}

fn edit_map_from_rows(rows: Vec<(VoxelCoord, u16)>) -> Result<EditMap, JsValue> {
    let mut edits = EditMap::default();
    for (coord, id) in rows {
        let material = Material::from_id(id)
            .ok_or_else(|| JsValue::from_str(&format!("unknown material id {id}")))?;
        edits.insert_override(coord, material);
    }
    Ok(edits)
}

fn encode_operation(message: &js_sys::Array, operation: &Operation) {
    match operation {
        Operation::LoadCamera => push_number(message, OP_LOAD_CAMERA),
        Operation::LoadEdits => push_number(message, OP_LOAD_EDITS),
        Operation::SaveCamera(values) => {
            push_number(message, OP_SAVE_CAMERA);
            for value in values {
                push_number(message, f64::from(*value));
            }
        }
        Operation::SaveEdit { coord, material } => {
            push_number(message, OP_SAVE_EDIT);
            push_number(message, f64::from(coord.x));
            push_number(message, f64::from(coord.y));
            push_number(message, f64::from(coord.z));
            push_number(message, material.map_or(-1.0, f64::from));
        }
    }
}

fn decode_operation(message: &js_sys::Array) -> Result<Operation, String> {
    match number(message, 5) {
        Some(OP_LOAD_CAMERA) => Ok(Operation::LoadCamera),
        Some(OP_LOAD_EDITS) => Ok(Operation::LoadEdits),
        Some(OP_SAVE_CAMERA) => Ok(Operation::SaveCamera([
            finite_f32(message, 6)?,
            finite_f32(message, 7)?,
            finite_f32(message, 8)?,
            finite_f32(message, 9)?,
            finite_f32(message, 10)?,
        ])),
        Some(OP_SAVE_EDIT) => Ok(Operation::SaveEdit {
            coord: decode_coord(message, 6)?,
            material: decode_optional_material(message, 9)?,
        }),
        _ => Err("unknown persistence operation".into()),
    }
}

fn decode_coord(message: &js_sys::Array, offset: u32) -> Result<VoxelCoord, String> {
    Ok(VoxelCoord::new(
        finite_i32(message, offset)?,
        finite_i32(message, offset + 1)?,
        finite_i32(message, offset + 2)?,
    ))
}

fn decode_optional_material(message: &js_sys::Array, index: u32) -> Result<Option<u16>, String> {
    let material = finite_number(message, index)?;
    if material == -1.0 {
        Ok(None)
    } else if material.fract() == 0.0 && (0.0..=f64::from(u16::MAX)).contains(&material) {
        Ok(Some(material as u16))
    } else {
        Err("invalid material id in persistence message".into())
    }
}

fn encode_result(message: &js_sys::Array, result: &OperationResult) {
    match result {
        OperationResult::Ok => push_number(message, RESULT_OK),
        OperationResult::Error(error) => {
            push_number(message, RESULT_ERROR);
            message.push(&JsValue::from_str(error));
        }
        OperationResult::Camera(None) => push_number(message, RESULT_NO_CAMERA),
        OperationResult::Camera(Some(values)) => {
            push_number(message, RESULT_CAMERA);
            for value in values {
                push_number(message, f64::from(*value));
            }
        }
        OperationResult::Edits(rows) => {
            push_number(message, RESULT_EDITS);
            push_number(message, rows.len() as f64);
            for (coord, material) in rows {
                push_number(message, f64::from(coord.x));
                push_number(message, f64::from(coord.y));
                push_number(message, f64::from(coord.z));
                push_number(message, f64::from(*material));
            }
        }
    }
}

fn decode_result(message: &js_sys::Array) -> Option<OperationResult> {
    match number(message, 4)? {
        RESULT_OK => Some(OperationResult::Ok),
        RESULT_ERROR => Some(OperationResult::Error(message.get(5).as_string()?)),
        RESULT_NO_CAMERA => Some(OperationResult::Camera(None)),
        RESULT_CAMERA => Some(OperationResult::Camera(Some([
            finite_f32(message, 5).ok()?,
            finite_f32(message, 6).ok()?,
            finite_f32(message, 7).ok()?,
            finite_f32(message, 8).ok()?,
            finite_f32(message, 9).ok()?,
        ]))),
        RESULT_EDITS => {
            let count = usize::try_from(integer(message, 5)?).ok()?;
            let expected = 6usize.checked_add(count.checked_mul(4)?)?;
            if message.length() as usize != expected {
                return None;
            }
            let mut rows = Vec::with_capacity(count);
            for index in 0..count {
                let offset = 6 + index * 4;
                rows.push((
                    VoxelCoord::new(
                        finite_i32(message, offset as u32).ok()?,
                        finite_i32(message, (offset + 1) as u32).ok()?,
                        finite_i32(message, (offset + 2) as u32).ok()?,
                    ),
                    finite_u16(message, (offset + 3) as u32).ok()?,
                ));
            }
            Some(OperationResult::Edits(rows))
        }
        _ => None,
    }
}

fn operation_result(result: OperationResult) -> Result<(), JsValue> {
    match result {
        OperationResult::Ok => Ok(()),
        OperationResult::Error(error) => Err(JsValue::from_str(&error)),
        _ => Err(JsValue::from_str(
            "persistence write returned an invalid response",
        )),
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

fn finite_i32(message: &js_sys::Array, index: u32) -> Result<i32, String> {
    let value = finite_number(message, index)?;
    if value.fract() == 0.0 && value >= f64::from(i32::MIN) && value <= f64::from(i32::MAX) {
        Ok(value as i32)
    } else {
        Err("numeric wire value is outside i32 range".into())
    }
}

fn finite_u16(message: &js_sys::Array, index: u32) -> Result<u16, String> {
    let value = finite_number(message, index)?;
    if value.fract() == 0.0 && (0.0..=f64::from(u16::MAX)).contains(&value) {
        Ok(value as u16)
    } else {
        Err("numeric wire value is outside u16 range".into())
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

fn migrate(connection: &Connection) -> Result<(), JsValue> {
    let mut version: i64 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(|error| js_error("read schema version", error))?;
    if version > SCHEMA_VERSION {
        return Err(JsValue::from_str(&format!(
            "local database schema {version} is newer than this build ({SCHEMA_VERSION})"
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
                 CREATE TABLE camera (
                   id INTEGER PRIMARY KEY CHECK (id = 0),
                   x REAL NOT NULL,
                   y REAL NOT NULL,
                   z REAL NOT NULL,
                   yaw REAL NOT NULL,
                   pitch REAL NOT NULL
                 );
                 CREATE TABLE chunks (
                   world_id INTEGER NOT NULL REFERENCES worlds(id) ON DELETE CASCADE,
                   x INTEGER NOT NULL,
                   y INTEGER NOT NULL,
                   z INTEGER NOT NULL,
                   revision INTEGER NOT NULL,
                   codec_version INTEGER NOT NULL,
                   payload BLOB NOT NULL,
                   content_hash BLOB NOT NULL CHECK (length(content_hash) = 32),
                   updated_at INTEGER NOT NULL DEFAULT (unixepoch()),
                   PRIMARY KEY (world_id, x, y, z)
                 ) WITHOUT ROWID;
                 CREATE TABLE voxel_edits (
                   world_id INTEGER NOT NULL REFERENCES worlds(id) ON DELETE CASCADE,
                   x INTEGER NOT NULL,
                   y INTEGER NOT NULL,
                   z INTEGER NOT NULL,
                   material INTEGER NOT NULL,
                   updated_at INTEGER NOT NULL DEFAULT (unixepoch()),
                   PRIMARY KEY (world_id, x, y, z)
                 ) WITHOUT ROWID;
                 PRAGMA user_version=3;
                 COMMIT;",
            )
            .map_err(|error| js_error("apply schema migration 1", error))?;
        version = 3;
    }
    if version == 1 {
        // Schema 1 stored positions in whole-voxel units. Schema 2 uses SI metres so that the
        // 10 cm voxel resolution is explicit throughout simulation, rendering and persistence.
        connection
            .execute_batch(
                "BEGIN IMMEDIATE;
                 UPDATE camera SET x=x*0.1, y=y*0.1, z=z*0.1;
                 PRAGMA user_version=2;
                 COMMIT;",
            )
            .map_err(|error| js_error("apply schema migration 2", error))?;
        version = 2;
    }
    if version == 2 {
        connection
            .execute_batch(
                "BEGIN IMMEDIATE;
                 CREATE TABLE voxel_edits (
                   world_id INTEGER NOT NULL REFERENCES worlds(id) ON DELETE CASCADE,
                   x INTEGER NOT NULL,
                   y INTEGER NOT NULL,
                   z INTEGER NOT NULL,
                   material INTEGER NOT NULL,
                   updated_at INTEGER NOT NULL DEFAULT (unixepoch()),
                   PRIMARY KEY (world_id, x, y, z)
                 ) WITHOUT ROWID;
                 PRAGMA user_version=3;
                 COMMIT;",
            )
            .map_err(|error| js_error("apply schema migration 3", error))?;
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
        if seed.as_slice() != world_seed.to_le_bytes() || version > generator_version {
            return Err(JsValue::from_str(
                "saved world identity does not match this generator build",
            ));
        }
        if version < generator_version {
            connection
                .execute(
                    "UPDATE worlds SET generator_version=?1 WHERE id=0",
                    params![generator_version],
                )
                .map_err(|error| js_error("advance world generator version", error))?;
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
