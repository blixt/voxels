//! Dedicated browser-worker transport for latency-sensitive player presence.

use glam::Vec3;
use js_sys::{Array, ArrayBuffer, Function, Promise, Uint8Array};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use voxels_client_config::{MultiplayerConfig, WorldTransportConfig};
use voxels_core::{
    CameraState, LocomotionMode, PresenceInterpolationConfig, RemoteAvatarPose, RemotePlayerId,
    RemotePoseSample, RemotePoseUpdate, RemotePresenceDelta, RemotePresenceTimeline,
};
use voxels_world::protocol::{
    self, OpenPresence, PLAYER_POSE_FLYING, PLAYER_POSE_GLIDING, PLAYER_POSE_GROUNDED,
    PLAYER_POSE_SWIMMING, PlayerId, PlayerPoseUpdate, PresencePing, PresencePong,
    PresenceSessionId, WorldOpened,
};
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
use wasm_bindgen_futures::JsFuture;
use web_sys::{BinaryType, CloseEvent, Event, MessageEvent, WebSocket, WorkerGlobalScope};

const RECONNECT_DELAY_MS: f64 = 500.0;
const GRACEFUL_CLOSE_TIMEOUT_MS: i32 = 250;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PresenceConnectionState {
    Connecting,
    Handshaking,
    Open,
    WaitingToReconnect,
    Closed,
}

#[derive(Clone)]
pub struct RemotePresenceClient {
    inner: Rc<PresenceInner>,
}

impl RemotePresenceClient {
    pub fn start(
        transport: WorldTransportConfig,
        config: MultiplayerConfig,
        opened: &WorldOpened,
    ) -> Result<Self, String> {
        transport.validate().map_err(|error| error.to_string())?;
        config.validate().map_err(|error| error.to_string())?;
        if !opened
            .capabilities
            .contains(protocol::WorldCapabilities::PLAYER_PRESENCE)
        {
            return Err("world service lacks player-presence capability".to_owned());
        }
        let interpolation = PresenceInterpolationConfig {
            initial_delay_ms: u16::try_from(config.interpolation_delay_ms)
                .map_err(|_| "interpolation delay does not fit the client")?,
            min_delay_ms: u16::try_from(config.min_interpolation_delay_ms)
                .map_err(|_| "minimum interpolation delay does not fit the client")?,
            max_delay_ms: u16::try_from(config.max_interpolation_delay_ms)
                .map_err(|_| "maximum interpolation delay does not fit the client")?,
            max_extrapolation_ms: u16::try_from(config.max_extrapolation_ms)
                .map_err(|_| "extrapolation delay does not fit the client")?,
        };
        let timeline = RemotePresenceTimeline::new(interpolation)
            .map_err(|error| format!("presence interpolation: {error}"))?;
        let clock =
            ClockSync::from_server_sample(local_now_ms(), opened.environment.sample_server_time_ms);
        let inner = Rc::new(PresenceInner {
            transport,
            config,
            local_player_id: opened.identity.player_id,
            state: Cell::new(PresenceConnectionState::WaitingToReconnect),
            socket: RefCell::new(None),
            handlers: RefCell::new(None),
            generation: Cell::new(0),
            connection_id: Cell::new(opened.connection_id),
            session_id: Cell::new(Some(opened.presence_session_id)),
            next_pose_sequence: Cell::new(1),
            next_ping_sequence: Cell::new(1),
            last_pose_send_ms: Cell::new(f64::NEG_INFINITY),
            last_ping_send_ms: Cell::new(f64::NEG_INFINITY),
            reconnect_after_ms: Cell::new(0.0),
            clock: Cell::new(clock),
            timeline: RefCell::new(timeline),
            last_error: RefCell::new(None),
            close_resolver: RefCell::new(None),
        });
        PresenceInner::open_socket(&inner)?;
        Ok(Self { inner })
    }

    /// Rebinds after a world-socket reconnect and otherwise performs bounded presence reconnects.
    pub fn ensure_session(&self, opened: &WorldOpened, local_time_ms: f64) {
        if opened.connection_id != self.inner.connection_id.get()
            || self.inner.session_id.get() != Some(opened.presence_session_id)
        {
            self.inner.replace_session(opened, local_time_ms);
        }
        if self.inner.state.get() == PresenceConnectionState::WaitingToReconnect
            && local_time_ms >= self.inner.reconnect_after_ms.get()
            && let Err(error) = PresenceInner::open_socket(&self.inner)
        {
            *self.inner.last_error.borrow_mut() = Some(error);
            self.inner
                .reconnect_after_ms
                .set(local_time_ms + RECONNECT_DELAY_MS);
        }
    }

    /// Coalesces local pose transmission and clock sync on the render clock, then samples remote
    /// players from the shared delayed server timeline.
    pub fn update(
        &self,
        camera: &CameraState,
        local_time_ms: f64,
        frame_delta_seconds: f32,
        send_local_pose: bool,
    ) -> Vec<RemoteAvatarPose> {
        if self.inner.state.get() == PresenceConnectionState::Open {
            self.inner.maybe_send_ping(local_time_ms);
            if send_local_pose {
                self.inner.maybe_send_pose(camera, local_time_ms);
            }
        }
        let estimated_server_time = self.inner.clock.get().server_time(local_time_ms);
        self.inner
            .timeline
            .borrow_mut()
            .sample(estimated_server_time, frame_delta_seconds)
    }

    pub fn take_error(&self) -> Option<String> {
        self.inner.last_error.borrow_mut().take()
    }

    pub fn estimated_server_time_ms(&self, local_time_ms: f64) -> f64 {
        self.inner.clock.get().server_time(local_time_ms)
    }

    pub fn close(&self) {
        self.inner.close();
    }

    /// Sends the final camera after the normal pose-rate interval, then waits for the server to
    /// acknowledge the WebSocket close before the owning world session is allowed to checkpoint.
    pub async fn close_after_final_pose(&self, camera: &CameraState) {
        self.inner.close_after_final_pose(camera).await;
    }
}

impl Drop for RemotePresenceClient {
    fn drop(&mut self) {
        if Rc::strong_count(&self.inner) == 1 {
            self.inner.close();
        }
    }
}

struct PresenceInner {
    transport: WorldTransportConfig,
    config: MultiplayerConfig,
    local_player_id: PlayerId,
    state: Cell<PresenceConnectionState>,
    socket: RefCell<Option<WebSocket>>,
    handlers: RefCell<Option<PresenceSocketHandlers>>,
    generation: Cell<u64>,
    connection_id: Cell<u64>,
    session_id: Cell<Option<PresenceSessionId>>,
    next_pose_sequence: Cell<u64>,
    next_ping_sequence: Cell<u32>,
    last_pose_send_ms: Cell<f64>,
    last_ping_send_ms: Cell<f64>,
    reconnect_after_ms: Cell<f64>,
    clock: Cell<ClockSync>,
    timeline: RefCell<RemotePresenceTimeline>,
    last_error: RefCell<Option<String>>,
    close_resolver: RefCell<Option<Function>>,
}

struct PresenceSocketHandlers {
    _open: Closure<dyn FnMut(Event)>,
    _message: Closure<dyn FnMut(MessageEvent)>,
    _error: Closure<dyn FnMut(Event)>,
    _close: Closure<dyn FnMut(CloseEvent)>,
}

#[derive(Clone, Copy)]
struct ClockSync {
    offset_ms: f64,
    best_round_trip_ms: f64,
    latest_round_trip_ms: u32,
    synchronized: bool,
}

impl Default for ClockSync {
    fn default() -> Self {
        Self {
            offset_ms: 0.0,
            best_round_trip_ms: f64::INFINITY,
            latest_round_trip_ms: 0,
            synchronized: false,
        }
    }
}

impl ClockSync {
    fn from_server_sample(local_receive_ms: f64, server_time_ms: u64) -> Self {
        let mut clock = Self::default();
        clock.observe_opened(local_receive_ms, server_time_ms);
        clock
    }

    fn server_time(self, local_time_ms: f64) -> f64 {
        local_time_ms + self.offset_ms
    }

    fn observe_opened(&mut self, local_receive_ms: f64, server_time_ms: u64) {
        self.offset_ms = server_time_ms as f64 - local_receive_ms;
        self.synchronized = true;
    }

    fn observe_pong(&mut self, local_receive_ms: f64, pong: PresencePong) {
        let local_send_ms = pong.client_send_time_ms as f64;
        let server_processing_ms = pong
            .server_send_time_ms
            .saturating_sub(pong.server_receive_time_ms) as f64;
        let round_trip_ms = (local_receive_ms - local_send_ms - server_processing_ms).max(0.0);
        self.latest_round_trip_ms = round_trip_ms.round().clamp(1.0, u32::MAX as f64) as u32;
        let measured_offset = ((pong.server_receive_time_ms as f64 - local_send_ms)
            + (pong.server_send_time_ms as f64 - local_receive_ms))
            * 0.5;
        let preferred = round_trip_ms <= self.best_round_trip_ms + 5.0;
        self.best_round_trip_ms = self.best_round_trip_ms.min(round_trip_ms);
        let alpha = if !self.synchronized {
            1.0
        } else if preferred {
            0.25
        } else {
            0.04
        };
        self.offset_ms += (measured_offset - self.offset_ms) * alpha;
        self.synchronized = true;
    }
}

impl PresenceInner {
    fn open_socket(inner: &Rc<Self>) -> Result<(), String> {
        if inner.state.get() == PresenceConnectionState::Closed {
            return Err("presence client is closed".to_owned());
        }
        let Some(_session_id) = inner.session_id.get() else {
            return Err("presence session is unavailable".to_owned());
        };
        inner.detach_socket(1001, "presence reconnect");
        let protocols = Array::new();
        protocols.push(&inner.transport.subprotocol.clone().into());
        protocols.push(&inner.transport.auth_subprotocol_token.clone().into());
        let socket = WebSocket::new_with_str_sequence(
            &inner.transport.presence_endpoint,
            protocols.as_ref(),
        )
        .map_err(js_reason)?;
        socket.set_binary_type(BinaryType::Arraybuffer);
        let generation = inner.generation.get().wrapping_add(1);
        inner.generation.set(generation);
        inner.state.set(PresenceConnectionState::Connecting);

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
                        "presence server sent a non-binary message".to_owned(),
                    ),
                }
            }
        });
        let weak = Rc::downgrade(inner);
        let error = Closure::new(move |_event: Event| {
            if let Some(inner) = weak.upgrade() {
                inner.disconnect(generation, "presence WebSocket error".to_owned());
            }
        });
        let weak = Rc::downgrade(inner);
        let close = Closure::new(move |event: CloseEvent| {
            if let Some(inner) = weak.upgrade() {
                inner.handle_close(generation, event.code());
            }
        });
        socket.set_onopen(Some(open.as_ref().unchecked_ref()));
        socket.set_onmessage(Some(message.as_ref().unchecked_ref()));
        socket.set_onerror(Some(error.as_ref().unchecked_ref()));
        socket.set_onclose(Some(close.as_ref().unchecked_ref()));
        *inner.socket.borrow_mut() = Some(socket);
        *inner.handlers.borrow_mut() = Some(PresenceSocketHandlers {
            _open: open,
            _message: message,
            _error: error,
            _close: close,
        });
        Ok(())
    }

    fn handle_open(&self, generation: u64) {
        if generation != self.generation.get()
            || self.state.get() != PresenceConnectionState::Connecting
        {
            return;
        }
        let Some(socket) = self.socket.borrow().clone() else {
            return;
        };
        if socket.protocol() != self.transport.subprotocol {
            self.disconnect(
                generation,
                "presence server negotiated a different subprotocol".to_owned(),
            );
            return;
        }
        let Some(session_id) = self.session_id.get() else {
            return;
        };
        let frame = match protocol::encode_open_presence(OpenPresence { session_id }) {
            Ok(frame) => frame,
            Err(error) => {
                self.disconnect(generation, error.to_string());
                return;
            }
        };
        if let Err(error) = socket.send_with_u8_array(&frame) {
            self.disconnect(generation, js_reason(error));
            return;
        }
        self.state.set(PresenceConnectionState::Handshaking);
    }

    fn handle_message(&self, generation: u64, bytes: Vec<u8>) {
        if generation != self.generation.get() {
            return;
        }
        let kind = match protocol::message_kind(&bytes) {
            Ok(kind) => kind,
            Err(error) => {
                self.disconnect(generation, error.to_string());
                return;
            }
        };
        if kind == protocol::presence_opened_kind() {
            let opened = match protocol::decode_presence_opened(&bytes) {
                Ok(opened) => opened,
                Err(error) => {
                    self.disconnect(generation, error.to_string());
                    return;
                }
            };
            if self.state.get() != PresenceConnectionState::Handshaking
                || opened.connection_id != self.connection_id.get()
            {
                self.disconnect(generation, "presence connection id mismatch".to_owned());
                return;
            }
            let now = local_now_ms();
            let mut clock = self.clock.get();
            clock.observe_opened(now, opened.server_time_ms);
            self.clock.set(clock);
            self.state.set(PresenceConnectionState::Open);
            self.last_ping_send_ms.set(f64::NEG_INFINITY);
            self.last_pose_send_ms.set(f64::NEG_INFINITY);
        } else if kind == protocol::presence_delta_kind() {
            if self.state.get() != PresenceConnectionState::Open {
                self.disconnect(
                    generation,
                    "presence delta arrived before handshake".to_owned(),
                );
                return;
            }
            let delta = match protocol::decode_presence_delta(&bytes) {
                Ok(delta) => delta,
                Err(error) => {
                    self.disconnect(generation, error.to_string());
                    return;
                }
            };
            let local_player_id = self.local_player_id;
            let enters = delta
                .enters
                .into_iter()
                .filter(|player| player.player_id != local_player_id)
                .map(|player| RemotePoseSample {
                    player_id: RemotePlayerId(*player.player_id.as_bytes()),
                    connection_id: player.connection_id,
                    color_index: player.color_index,
                    sequence: player.pose.sequence,
                    sample_server_time_ms: player.pose.sample_server_time_ms,
                    eye_position_metres: Vec3::from_array(player.pose.eye_position_metres),
                    linear_velocity_metres_per_second: Vec3::from_array(
                        player.pose.linear_velocity_metres_per_second,
                    ),
                    look_yaw_radians: player.pose.look_yaw_radians,
                    look_pitch_radians: player.pose.look_pitch_radians,
                    flags: player.pose.flags,
                })
                .collect();
            let updates = delta
                .updates
                .into_iter()
                .filter(|update| update.connection_id != self.connection_id.get())
                .map(|update| RemotePoseUpdate {
                    connection_id: update.connection_id,
                    sequence: update.pose.sequence,
                    sample_server_time_ms: update.pose.sample_server_time_ms,
                    eye_position_metres: Vec3::from_array(update.pose.eye_position_metres),
                    linear_velocity_metres_per_second: Vec3::from_array(
                        update.pose.linear_velocity_metres_per_second,
                    ),
                    look_yaw_radians: update.pose.look_yaw_radians,
                    look_pitch_radians: update.pose.look_pitch_radians,
                    flags: update.pose.flags,
                })
                .collect();
            self.timeline.borrow_mut().ingest_delta(
                RemotePresenceDelta {
                    stream_sequence: delta.stream_sequence,
                    server_time_ms: delta.server_time_ms,
                    enters,
                    updates,
                    leaves: delta.leaves,
                },
                local_now_ms(),
            );
        } else if kind == protocol::presence_pong_kind() {
            let pong = match protocol::decode_presence_pong(&bytes) {
                Ok(pong) => pong,
                Err(error) => {
                    self.disconnect(generation, error.to_string());
                    return;
                }
            };
            let mut clock = self.clock.get();
            clock.observe_pong(local_now_ms(), pong);
            self.clock.set(clock);
        } else if kind == protocol::error_kind() {
            let message = protocol::decode_error(&bytes)
                .map(|(_, message)| message)
                .unwrap_or_else(|error| error.to_string());
            self.disconnect(generation, message);
        } else {
            self.disconnect(generation, "unexpected presence server message".to_owned());
        }
    }

    fn maybe_send_pose(&self, camera: &CameraState, local_time_ms: f64) {
        if local_time_ms - self.last_pose_send_ms.get()
            < f64::from(self.config.pose_send_interval_ms)
        {
            return;
        }
        self.send_pose(camera, local_time_ms);
    }

    fn send_pose(&self, camera: &CameraState, local_time_ms: f64) {
        let Some(socket) = self.socket.borrow().clone() else {
            return;
        };
        if socket.buffered_amount() >= self.config.buffered_amount_high_water_bytes {
            return;
        }
        let mut flags = 0;
        if camera.grounded {
            flags |= PLAYER_POSE_GROUNDED;
        }
        if camera.fluid_state().swimming {
            flags |= PLAYER_POSE_SWIMMING;
        }
        if camera.locomotion() == LocomotionMode::CreativeFlight {
            flags |= PLAYER_POSE_FLYING;
        }
        if camera.locomotion() == LocomotionMode::Gliding {
            flags |= PLAYER_POSE_GLIDING;
        }
        let sequence = self.next_pose_sequence.get().max(1);
        let clock = self.clock.get();
        let sample_server_time_ms = if clock.synchronized {
            finite_milliseconds(clock.server_time(local_time_ms))
        } else {
            0
        };
        let frame = match protocol::encode_player_pose(PlayerPoseUpdate {
            sequence,
            sample_server_time_ms,
            eye_position_metres: camera.position.to_array(),
            linear_velocity_metres_per_second: camera.velocity.to_array(),
            look_yaw_radians: camera.yaw,
            look_pitch_radians: camera.pitch,
            flags,
        }) {
            Ok(frame) => frame,
            Err(error) => {
                *self.last_error.borrow_mut() = Some(error.to_string());
                return;
            }
        };
        if let Err(error) = socket.send_with_u8_array(&frame) {
            self.disconnect(self.generation.get(), js_reason(error));
            return;
        }
        self.next_pose_sequence.set(sequence.wrapping_add(1).max(1));
        self.last_pose_send_ms.set(local_time_ms);
    }

    fn maybe_send_ping(&self, local_time_ms: f64) {
        if local_time_ms - self.last_ping_send_ms.get()
            < f64::from(self.config.clock_sync_interval_ms)
        {
            return;
        }
        let Some(socket) = self.socket.borrow().clone() else {
            return;
        };
        if socket.buffered_amount() >= self.config.buffered_amount_high_water_bytes {
            return;
        }
        let sequence = self.next_ping_sequence.get().max(1);
        let frame = match protocol::encode_presence_ping(PresencePing {
            sequence,
            observed_round_trip_ms: self.clock.get().latest_round_trip_ms,
            client_send_time_ms: finite_milliseconds(local_time_ms),
        }) {
            Ok(frame) => frame,
            Err(error) => {
                *self.last_error.borrow_mut() = Some(error.to_string());
                return;
            }
        };
        if let Err(error) = socket.send_with_u8_array(&frame) {
            self.disconnect(self.generation.get(), js_reason(error));
            return;
        }
        self.next_ping_sequence.set(sequence.wrapping_add(1).max(1));
        self.last_ping_send_ms.set(local_time_ms);
    }

    fn replace_session(&self, opened: &WorldOpened, local_time_ms: f64) {
        self.detach_socket(1001, "world session replaced");
        self.connection_id.set(opened.connection_id);
        self.session_id.set(Some(opened.presence_session_id));
        self.next_pose_sequence.set(1);
        self.next_ping_sequence.set(1);
        self.clock.set(ClockSync::from_server_sample(
            local_time_ms,
            opened.environment.sample_server_time_ms,
        ));
        self.timeline.borrow_mut().clear();
        self.state.set(PresenceConnectionState::WaitingToReconnect);
        self.reconnect_after_ms.set(0.0);
    }

    fn disconnect(&self, generation: u64, reason: String) {
        if generation != self.generation.get()
            || self.state.get() == PresenceConnectionState::Closed
        {
            return;
        }
        self.detach_socket(1011, "presence connection reset");
        self.timeline.borrow_mut().clear();
        self.state.set(PresenceConnectionState::WaitingToReconnect);
        self.reconnect_after_ms
            .set(local_now_ms() + RECONNECT_DELAY_MS);
        *self.last_error.borrow_mut() = Some(reason);
    }

    fn handle_close(&self, generation: u64, code: u16) {
        if generation != self.generation.get() {
            return;
        }
        if self.state.get() == PresenceConnectionState::Closed {
            self.socket.borrow_mut().take();
            self.handlers.borrow_mut().take();
            self.resolve_close();
        } else {
            self.disconnect(
                generation,
                format!("presence WebSocket closed with code {code}"),
            );
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
    }

    fn close(&self) {
        if self.state.replace(PresenceConnectionState::Closed) == PresenceConnectionState::Closed {
            return;
        }
        self.generation.set(self.generation.get().wrapping_add(1));
        self.detach_socket(1000, "presence client closed");
        self.resolve_close();
        self.timeline.borrow_mut().clear();
    }

    async fn close_after_final_pose(&self, camera: &CameraState) {
        if self.state.get() != PresenceConnectionState::Open {
            self.close();
            return;
        }
        let delay_ms = final_pose_delay_ms(
            local_now_ms(),
            self.last_pose_send_ms.get(),
            self.config.pose_send_interval_ms,
        );
        if delay_ms != 0 {
            let _ = JsFuture::from(timeout(delay_ms)).await;
        }
        if self.state.get() != PresenceConnectionState::Open {
            self.close();
            return;
        }
        self.send_pose(camera, local_now_ms());
        let (closed, resolve) = resolvable();
        *self.close_resolver.borrow_mut() = Some(resolve);
        self.state.set(PresenceConnectionState::Closed);
        if let Some(socket) = self.socket.borrow().as_ref() {
            let _ = socket.close_with_code_and_reason(1000, "presence client closed");
        } else {
            self.resolve_close();
        }
        let deadline = timeout(GRACEFUL_CLOSE_TIMEOUT_MS);
        let _ = JsFuture::from(Promise::race(&Array::of2(&closed, &deadline))).await;
        self.generation.set(self.generation.get().wrapping_add(1));
        self.detach_socket(1000, "presence close timeout");
        self.resolve_close();
        self.timeline.borrow_mut().clear();
    }

    fn resolve_close(&self) {
        if let Some(resolve) = self.close_resolver.borrow_mut().take() {
            let _ = resolve.call0(&wasm_bindgen::JsValue::UNDEFINED);
        }
    }
}

fn final_pose_delay_ms(local_time_ms: f64, last_pose_send_ms: f64, interval_ms: u32) -> i32 {
    let remaining = f64::from(interval_ms) - (local_time_ms - last_pose_send_ms);
    remaining.ceil().clamp(0.0, f64::from(i32::MAX)) as i32
}

fn timeout(milliseconds: i32) -> Promise {
    Promise::new(&mut |resolve, _reject| {
        let scope: WorkerGlobalScope = js_sys::global().unchecked_into();
        let _ = scope.set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, milliseconds);
    })
}

fn resolvable() -> (Promise, Function) {
    let slot = Rc::new(RefCell::new(None));
    let captured = Rc::clone(&slot);
    let promise = Promise::new(&mut |resolve, _reject| {
        *captured.borrow_mut() = Some(resolve);
    });
    let resolve = match slot.borrow_mut().take() {
        Some(resolve) => resolve,
        None => Function::new_no_args(""),
    };
    (promise, resolve)
}

fn local_now_ms() -> f64 {
    let scope: WorkerGlobalScope = js_sys::global().unchecked_into();
    scope
        .performance()
        .map_or(0.0, |performance| performance.now())
}

fn finite_milliseconds(value: f64) -> u64 {
    if value.is_finite() {
        value.round().clamp(1.0, u64::MAX as f64) as u64
    } else {
        1
    }
}

fn js_reason(value: wasm_bindgen::JsValue) -> String {
    value
        .as_string()
        .unwrap_or_else(|| "presence JavaScript exception".to_owned())
}

#[cfg(test)]
mod tests {
    use super::{ClockSync, final_pose_delay_ms};

    #[test]
    fn final_pose_waits_only_for_the_unsatisfied_send_interval() {
        assert_eq!(final_pose_delay_ms(100.0, 90.0, 33), 23);
        assert_eq!(final_pose_delay_ms(123.0, 90.0, 33), 0);
        assert_eq!(final_pose_delay_ms(150.0, f64::NEG_INFINITY, 33), 0);
    }

    #[test]
    fn world_open_sample_seeds_server_time_before_the_first_pong() {
        let clock = ClockSync::from_server_sample(250.0, 10_000);

        assert_eq!(clock.server_time(275.0), 10_025.0);
        assert!(clock.synchronized);
    }
}
