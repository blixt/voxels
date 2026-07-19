use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use tokio::sync::Notify;
use tokio::time::{Duration, Instant};

/// Small enough to keep one non-preemptible write near the queue-delay target on constrained
/// links, while still amortizing VXWP fragment and WebSocket framing overhead.
const MIN_PACING_QUANTUM_BYTES: usize = 2 * 1024;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum TrafficPriority {
    Critical = 0,
    Collision = 1,
    WorldChange = 2,
    RealtimePresence = 3,
    VisibleWorld = 4,
    BackgroundWorld = 5,
}

impl TrafficPriority {
    pub(crate) const COUNT: usize = 6;
    pub(crate) const ALL: [Self; Self::COUNT] = [
        Self::Critical,
        Self::Collision,
        Self::WorldChange,
        Self::RealtimePresence,
        Self::VisibleWorld,
        Self::BackgroundWorld,
    ];
    const WEIGHTS: [f64; Self::COUNT] = [f64::INFINITY, 16.0, 12.0, 5.0, 3.0, 0.5];

    pub(crate) const fn index(self) -> usize {
        self as usize
    }
}

pub(crate) struct ClientTrafficShaper {
    floor_bytes_per_second: f64,
    ceiling_bytes_per_second: f64,
    burst_bytes: f64,
    queue_delay_target: Duration,
    feedback_timeout: Duration,
    max_frame_fragment_bytes: usize,
    state: Mutex<TrafficState>,
    changed: Notify,
}

pub(crate) struct ClientTrafficRegistry {
    floor_bytes_per_second: usize,
    ceiling_bytes_per_second: usize,
    burst_bytes: usize,
    queue_delay_target: Duration,
    feedback_timeout: Duration,
    max_frame_fragment_bytes: usize,
    clients: Mutex<HashMap<u64, Weak<ClientTrafficShaper>>>,
}

pub(crate) struct ClientTrafficRegistration {
    connection_id: u64,
    shaper: Arc<ClientTrafficShaper>,
    registry: Arc<ClientTrafficRegistry>,
}

struct TrafficState {
    tokens: f64,
    last_refill: Instant,
    current_bytes_per_second: f64,
    minimum_round_trip: Option<Duration>,
    previous_round_trip: Option<Duration>,
    last_feedback: Option<Instant>,
    rate_limited_since_feedback: bool,
    waiters: [usize; TrafficPriority::COUNT],
    virtual_service: [f64; TrafficPriority::COUNT],
}

impl ClientTrafficRegistry {
    pub(crate) fn new(
        floor_bytes_per_second: usize,
        ceiling_bytes_per_second: usize,
        burst_bytes: usize,
        queue_delay_target: Duration,
        feedback_timeout: Duration,
        max_frame_fragment_bytes: usize,
    ) -> Arc<Self> {
        Arc::new(Self {
            floor_bytes_per_second,
            ceiling_bytes_per_second,
            burst_bytes,
            queue_delay_target,
            feedback_timeout,
            max_frame_fragment_bytes,
            clients: Mutex::new(HashMap::new()),
        })
    }

    pub(crate) fn register(self: &Arc<Self>, connection_id: u64) -> ClientTrafficRegistration {
        let shaper = ClientTrafficShaper::new(
            self.floor_bytes_per_second,
            self.ceiling_bytes_per_second,
            self.burst_bytes,
            self.queue_delay_target,
            self.feedback_timeout,
            self.max_frame_fragment_bytes,
        );
        self.lock().insert(connection_id, Arc::downgrade(&shaper));
        ClientTrafficRegistration {
            connection_id,
            shaper,
            registry: Arc::clone(self),
        }
    }

    pub(crate) fn get(&self, connection_id: u64) -> Option<Arc<ClientTrafficShaper>> {
        self.lock().get(&connection_id).and_then(Weak::upgrade)
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<u64, Weak<ClientTrafficShaper>>> {
        match self.clients.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

impl ClientTrafficRegistration {
    pub(crate) fn shaper(&self) -> Arc<ClientTrafficShaper> {
        Arc::clone(&self.shaper)
    }
}

impl Drop for ClientTrafficRegistration {
    fn drop(&mut self) {
        let mut clients = self.registry.lock();
        let is_current = clients
            .get(&self.connection_id)
            .and_then(Weak::upgrade)
            .is_some_and(|current| Arc::ptr_eq(&current, &self.shaper));
        if is_current {
            clients.remove(&self.connection_id);
        }
    }
}

impl ClientTrafficShaper {
    pub(crate) fn new(
        floor_bytes_per_second: usize,
        ceiling_bytes_per_second: usize,
        burst_bytes: usize,
        queue_delay_target: Duration,
        feedback_timeout: Duration,
        max_frame_fragment_bytes: usize,
    ) -> Arc<Self> {
        let initial_burst_bytes = effective_burst_bytes(
            floor_bytes_per_second as f64,
            burst_bytes as f64,
            queue_delay_target,
        );
        Arc::new(Self {
            floor_bytes_per_second: floor_bytes_per_second as f64,
            ceiling_bytes_per_second: ceiling_bytes_per_second as f64,
            burst_bytes: burst_bytes as f64,
            queue_delay_target,
            feedback_timeout,
            max_frame_fragment_bytes,
            state: Mutex::new(TrafficState {
                tokens: initial_burst_bytes,
                last_refill: Instant::now(),
                current_bytes_per_second: floor_bytes_per_second as f64,
                minimum_round_trip: None,
                previous_round_trip: None,
                last_feedback: None,
                rate_limited_since_feedback: false,
                waiters: [0; TrafficPriority::COUNT],
                virtual_service: [0.0; TrafficPriority::COUNT],
            }),
            changed: Notify::new(),
        })
    }

    pub(crate) async fn acquire(self: &Arc<Self>, priority: TrafficPriority, bytes: usize) {
        if bytes == 0 {
            return;
        }
        {
            let mut state = self.lock();
            let effective_burst_bytes = state.prepare(
                Instant::now(),
                self.floor_bytes_per_second,
                self.burst_bytes,
                self.queue_delay_target,
                self.feedback_timeout,
            );
            if state.waiters.iter().all(|count| *count == 0)
                && state.has_tokens(bytes, effective_burst_bytes)
            {
                state.consume(priority, bytes);
                return;
            }
            state.rate_limited_since_feedback = true;
        }
        self.acquire_contended(priority, bytes).await;
    }

    pub(crate) async fn acquire_contended(
        self: &Arc<Self>,
        priority: TrafficPriority,
        bytes: usize,
    ) {
        if bytes == 0 {
            return;
        }
        let mut waiter = TrafficWaiter::new(Arc::clone(self), priority);
        loop {
            let notified = self.changed.notified();
            let delay = {
                let mut state = self.lock();
                let effective_burst_bytes = state.prepare(
                    Instant::now(),
                    self.floor_bytes_per_second,
                    self.burst_bytes,
                    self.queue_delay_target,
                    self.feedback_timeout,
                );
                state.rate_limited_since_feedback = true;
                if state.can_send(priority, bytes, effective_burst_bytes) {
                    state.consume(priority, bytes);
                    waiter.complete(&mut state);
                    if state.waiters.iter().any(|count| *count > 0) {
                        self.changed.notify_waiters();
                    }
                    return;
                }
                state.delay(
                    priority,
                    bytes,
                    state.current_bytes_per_second,
                    effective_burst_bytes,
                )
            };
            tokio::select! {
                () = tokio::time::sleep(delay) => {}
                () = notified => {}
            }
        }
    }

    /// Applies receiver-observed end-to-end latency to the application pacing rate.
    ///
    /// TCP remains responsible for congestion control. This controller prevents the application
    /// from persistently filling transport and network buffers above it: demand may raise the rate
    /// after a low-delay sample, while one sample above the standing-queue target halves it.
    pub(crate) fn observe_round_trip(&self, round_trip: Duration) {
        if round_trip.is_zero() {
            return;
        }
        let now = Instant::now();
        let mut state = self.lock();
        state.prepare(
            now,
            self.floor_bytes_per_second,
            self.burst_bytes,
            self.queue_delay_target,
            self.feedback_timeout,
        );
        let previous_round_trip = state.previous_round_trip;
        let had_baseline = state.minimum_round_trip.is_some();
        let minimum = state
            .minimum_round_trip
            .map_or(round_trip, |minimum| minimum.min(round_trip));
        state.minimum_round_trip = Some(minimum);
        let queue_delay = round_trip.saturating_sub(minimum);
        let latency_is_rising = previous_round_trip.is_some_and(|previous| {
            round_trip.saturating_sub(previous) > self.queue_delay_target / 2
        });
        let previous_rate = state.current_bytes_per_second;
        let currently_backlogged = state.waiters.iter().any(|count| *count > 0);
        if queue_delay > self.queue_delay_target || latency_is_rising {
            state.current_bytes_per_second =
                (state.current_bytes_per_second * 0.5).max(self.floor_bytes_per_second);
        } else if had_baseline
            && state.rate_limited_since_feedback
            && currently_backlogged
            && queue_delay <= self.queue_delay_target / 4
        {
            state.current_bytes_per_second =
                (state.current_bytes_per_second * 1.15).min(self.ceiling_bytes_per_second);
        }
        state.previous_round_trip = Some(round_trip);
        state.last_feedback = Some(now);
        state.rate_limited_since_feedback = false;
        state.tokens = state.tokens.min(effective_burst_bytes(
            state.current_bytes_per_second,
            self.burst_bytes,
            self.queue_delay_target,
        ));
        let rate_changed = previous_rate != state.current_bytes_per_second;
        drop(state);
        if rate_changed {
            self.changed.notify_waiters();
        }
    }

    pub(crate) fn current_rate_bytes_per_second(&self) -> usize {
        self.lock()
            .current_bytes_per_second
            .round()
            .clamp(0.0, usize::MAX as f64) as usize
    }

    pub(crate) fn frame_fragment_bytes(&self, frame_bytes: usize) -> Option<usize> {
        let rate = self.lock().current_bytes_per_second;
        let delay_budget_bytes = (rate * self.queue_delay_target.as_secs_f64()).floor() as usize;
        let fragment_bytes = delay_budget_bytes
            .max(MIN_PACING_QUANTUM_BYTES)
            .min(self.max_frame_fragment_bytes);
        (frame_bytes > fragment_bytes).then_some(fragment_bytes)
    }

    fn lock(&self) -> MutexGuard<'_, TrafficState> {
        match self.state.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

fn effective_burst_bytes(
    bytes_per_second: f64,
    configured_burst_bytes: f64,
    queue_delay_target: Duration,
) -> f64 {
    configured_burst_bytes.min(
        (bytes_per_second * queue_delay_target.as_secs_f64()).max(MIN_PACING_QUANTUM_BYTES as f64),
    )
}

struct TrafficWaiter {
    shaper: Arc<ClientTrafficShaper>,
    priority: TrafficPriority,
    active: bool,
}

impl TrafficWaiter {
    fn new(shaper: Arc<ClientTrafficShaper>, priority: TrafficPriority) -> Self {
        let mut state = shaper.lock();
        let had_waiters = state.waiters.iter().any(|count| *count > 0);
        if !had_waiters {
            state.virtual_service = [0.0; TrafficPriority::COUNT];
        }
        state.waiters[priority.index()] += 1;
        drop(state);
        if had_waiters {
            shaper.changed.notify_waiters();
        }
        Self {
            shaper,
            priority,
            active: true,
        }
    }

    fn complete(&mut self, state: &mut TrafficState) {
        state.waiters[self.priority.index()] =
            state.waiters[self.priority.index()].saturating_sub(1);
        self.active = false;
    }
}

impl Drop for TrafficWaiter {
    fn drop(&mut self) {
        if !self.active {
            return;
        }
        let mut state = self.shaper.lock();
        state.waiters[self.priority.index()] =
            state.waiters[self.priority.index()].saturating_sub(1);
        let has_waiters = state.waiters.iter().any(|count| *count > 0);
        drop(state);
        if has_waiters {
            self.shaper.changed.notify_waiters();
        }
    }
}

impl TrafficState {
    fn prepare(
        &mut self,
        now: Instant,
        floor_bytes_per_second: f64,
        burst_bytes: f64,
        queue_delay_target: Duration,
        feedback_timeout: Duration,
    ) -> f64 {
        if self
            .last_feedback
            .is_some_and(|last| now.saturating_duration_since(last) > feedback_timeout)
        {
            self.current_bytes_per_second = floor_bytes_per_second;
            self.minimum_round_trip = None;
            self.previous_round_trip = None;
            self.last_feedback = None;
            self.rate_limited_since_feedback = false;
        }
        let effective_burst_bytes = effective_burst_bytes(
            self.current_bytes_per_second,
            burst_bytes,
            queue_delay_target,
        );
        self.refill(now, self.current_bytes_per_second, effective_burst_bytes);
        effective_burst_bytes
    }

    fn refill(&mut self, now: Instant, bytes_per_second: f64, burst_bytes: f64) {
        let elapsed = now.saturating_duration_since(self.last_refill);
        self.tokens = (self.tokens + elapsed.as_secs_f64() * bytes_per_second).min(burst_bytes);
        self.last_refill = now;
    }

    fn can_send(&self, priority: TrafficPriority, bytes: usize, burst_bytes: f64) -> bool {
        if priority != TrafficPriority::Critical
            && self.waiters[TrafficPriority::Critical.index()] > 0
        {
            return false;
        }
        if priority != TrafficPriority::Critical
            && self.selected_weighted_priority() != Some(priority)
        {
            return false;
        }
        self.has_tokens(bytes, burst_bytes)
    }

    fn has_tokens(&self, bytes: usize, burst_bytes: f64) -> bool {
        self.tokens >= (bytes as f64).min(burst_bytes)
    }

    fn consume(&mut self, priority: TrafficPriority, bytes: usize) {
        // Frames may be larger than the configured burst. Send one complete frame once the burst
        // is available, then retain the negative balance so later frames repay the whole debt.
        self.tokens -= bytes as f64;
        if priority != TrafficPriority::Critical {
            self.virtual_service[priority.index()] +=
                bytes as f64 / TrafficPriority::WEIGHTS[priority.index()];
        }
    }

    fn delay(
        &self,
        priority: TrafficPriority,
        bytes: usize,
        bytes_per_second: f64,
        burst_bytes: f64,
    ) -> Duration {
        if (priority != TrafficPriority::Critical
            && self.waiters[TrafficPriority::Critical.index()] > 0)
            || (priority != TrafficPriority::Critical
                && self.selected_weighted_priority() != Some(priority))
        {
            return Duration::from_secs(60);
        }
        let required = (bytes as f64).min(burst_bytes);
        Duration::from_secs_f64(
            ((required - self.tokens).max(0.0) / bytes_per_second).max(0.000_001),
        )
    }

    fn selected_weighted_priority(&self) -> Option<TrafficPriority> {
        [
            TrafficPriority::Collision,
            TrafficPriority::WorldChange,
            TrafficPriority::RealtimePresence,
            TrafficPriority::VisibleWorld,
            TrafficPriority::BackgroundWorld,
        ]
        .into_iter()
        .filter(|priority| self.waiters[priority.index()] > 0)
        .min_by(|left, right| {
            self.virtual_service[left.index()]
                .total_cmp(&self.virtual_service[right.index()])
                .then_with(|| left.cmp(right))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_bucket_retains_large_frame_debt() {
        let now = Instant::now();
        let mut state = TrafficState {
            tokens: 100.0,
            last_refill: now,
            current_bytes_per_second: 100.0,
            minimum_round_trip: None,
            previous_round_trip: None,
            last_feedback: None,
            rate_limited_since_feedback: false,
            waiters: [1, 0, 0, 0, 0, 0],
            virtual_service: [0.0; TrafficPriority::COUNT],
        };
        assert!(state.can_send(TrafficPriority::Critical, 250, 100.0));
        state.consume(TrafficPriority::Critical, 250);
        assert_eq!(state.tokens, -150.0);
        state.refill(now + Duration::from_secs(2), 100.0, 100.0);
        assert_eq!(state.tokens, 50.0);
    }

    #[test]
    fn critical_waiter_blocks_weighted_traffic() {
        let now = Instant::now();
        let state = TrafficState {
            tokens: 100.0,
            last_refill: now,
            current_bytes_per_second: 100.0,
            minimum_round_trip: None,
            previous_round_trip: None,
            last_feedback: None,
            rate_limited_since_feedback: false,
            waiters: [1, 0, 0, 0, 0, 1],
            virtual_service: [0.0; TrafficPriority::COUNT],
        };
        assert!(!state.can_send(TrafficPriority::BackgroundWorld, 10, 100.0));
        assert!(state.can_send(TrafficPriority::Critical, 10, 100.0));
    }

    #[test]
    fn weighted_classes_take_turns_by_normalized_bytes() {
        let now = Instant::now();
        let mut state = TrafficState {
            tokens: 10_000.0,
            last_refill: now,
            current_bytes_per_second: 10_000.0,
            minimum_round_trip: None,
            previous_round_trip: None,
            last_feedback: None,
            rate_limited_since_feedback: false,
            waiters: [0, 1, 1, 1, 1, 1],
            virtual_service: [0.0; TrafficPriority::COUNT],
        };
        assert_eq!(
            state.selected_weighted_priority(),
            Some(TrafficPriority::Collision)
        );
        state.consume(TrafficPriority::Collision, 1_600);
        assert_eq!(
            state.selected_weighted_priority(),
            Some(TrafficPriority::WorldChange)
        );
        state.consume(TrafficPriority::WorldChange, 1_200);
        assert_eq!(
            state.selected_weighted_priority(),
            Some(TrafficPriority::RealtimePresence)
        );
        state.consume(TrafficPriority::RealtimePresence, 500);
        assert_eq!(
            state.selected_weighted_priority(),
            Some(TrafficPriority::VisibleWorld)
        );
        state.consume(TrafficPriority::VisibleWorld, 300);
        assert_eq!(
            state.selected_weighted_priority(),
            Some(TrafficPriority::BackgroundWorld)
        );
        state.consume(TrafficPriority::BackgroundWorld, 50);
        assert_eq!(
            state.selected_weighted_priority(),
            Some(TrafficPriority::Collision)
        );
    }

    #[test]
    fn refill_never_exceeds_burst() {
        let now = Instant::now();
        let mut state = TrafficState {
            tokens: 20.0,
            last_refill: now,
            current_bytes_per_second: 100.0,
            minimum_round_trip: None,
            previous_round_trip: None,
            last_feedback: None,
            rate_limited_since_feedback: false,
            waiters: [0; TrafficPriority::COUNT],
            virtual_service: [0.0; TrafficPriority::COUNT],
        };
        state.refill(now + Duration::from_secs(10), 100.0, 200.0);
        assert_eq!(state.tokens, 200.0);
    }

    #[test]
    fn low_delay_feedback_raises_rate_only_when_demand_waited() {
        let shaper = ClientTrafficShaper::new(
            100,
            800,
            100,
            Duration::from_millis(20),
            Duration::from_secs(3),
            32 * 1024,
        );
        shaper.observe_round_trip(Duration::from_millis(40));
        assert_eq!(shaper.current_rate_bytes_per_second(), 100);
        {
            let mut state = shaper.lock();
            state.rate_limited_since_feedback = true;
            state.waiters[TrafficPriority::BackgroundWorld.index()] = 1;
        }
        shaper.observe_round_trip(Duration::from_millis(42));
        assert_eq!(shaper.current_rate_bytes_per_second(), 115);
    }

    #[test]
    fn standing_queue_feedback_cuts_rate_to_the_floor() {
        let shaper = ClientTrafficShaper::new(
            100,
            800,
            100,
            Duration::from_millis(20),
            Duration::from_secs(3),
            32 * 1024,
        );
        shaper.observe_round_trip(Duration::from_millis(40));
        {
            let mut state = shaper.lock();
            state.current_bytes_per_second = 150.0;
            state.rate_limited_since_feedback = true;
        }
        shaper.observe_round_trip(Duration::from_millis(70));
        assert_eq!(shaper.current_rate_bytes_per_second(), 100);
    }

    #[test]
    fn stale_feedback_restores_safe_floor_and_relearns_the_path() {
        let shaper = ClientTrafficShaper::new(
            100,
            800,
            100,
            Duration::from_millis(20),
            Duration::from_secs(3),
            32 * 1024,
        );
        {
            let mut state = shaper.lock();
            state.current_bytes_per_second = 800.0;
            state.minimum_round_trip = Some(Duration::from_millis(40));
            state.last_feedback = Some(Instant::now() - Duration::from_secs(4));
            state.prepare(
                Instant::now(),
                shaper.floor_bytes_per_second,
                shaper.burst_bytes,
                shaper.queue_delay_target,
                shaper.feedback_timeout,
            );
        }
        let state = shaper.lock();
        assert_eq!(state.current_bytes_per_second, 100.0);
        assert_eq!(state.minimum_round_trip, None);
        assert_eq!(state.previous_round_trip, None);
    }

    #[test]
    fn rising_latency_cuts_rate_even_when_the_first_baseline_was_queued() {
        let shaper = ClientTrafficShaper::new(
            100,
            800,
            100,
            Duration::from_millis(20),
            Duration::from_secs(3),
            32 * 1024,
        );
        shaper.observe_round_trip(Duration::from_millis(100));
        {
            let mut state = shaper.lock();
            state.current_bytes_per_second = 400.0;
            state.rate_limited_since_feedback = true;
            state.waiters[TrafficPriority::BackgroundWorld.index()] = 1;
        }
        shaper.observe_round_trip(Duration::from_millis(116));
        assert_eq!(shaper.current_rate_bytes_per_second(), 200);
    }

    #[test]
    fn pacing_burst_never_exceeds_one_queue_delay_target() {
        let shaper = ClientTrafficShaper::new(
            10_000,
            80_000,
            100_000,
            Duration::from_millis(200),
            Duration::from_secs(3),
            32 * 1024,
        );
        assert_eq!(shaper.lock().tokens, 2_048.0);
        {
            let mut state = shaper.lock();
            state.current_bytes_per_second = 80_000.0;
            state.tokens = 100_000.0;
            state.rate_limited_since_feedback = true;
        }
        shaper.observe_round_trip(Duration::from_millis(40));
        assert_eq!(shaper.lock().tokens, 16_000.0);
        {
            let mut state = shaper.lock();
            state.minimum_round_trip = Some(Duration::from_millis(40));
            state.tokens = 100_000.0;
        }
        shaper.observe_round_trip(Duration::from_millis(260));
        assert_eq!(shaper.current_rate_bytes_per_second(), 40_000);
        assert_eq!(shaper.lock().tokens, 8_000.0);
    }

    #[test]
    fn response_fragmentation_tracks_the_latency_budget_below_the_hard_cap() {
        let shaper = ClientTrafficShaper::new(
            100_000,
            800_000,
            64 * 1024,
            Duration::from_millis(25),
            Duration::from_secs(3),
            32 * 1024,
        );

        assert_eq!(shaper.frame_fragment_bytes(2_500), None);
        assert_eq!(shaper.frame_fragment_bytes(2_501), Some(2_500));
        assert_eq!(shaper.frame_fragment_bytes(12 * 1024), Some(2_500));
    }
}
