use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use tokio::sync::Notify;
use tokio::time::{Duration, Instant};

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
    state: Mutex<TrafficState>,
    changed: Notify,
}

pub(crate) struct ClientTrafficRegistry {
    floor_bytes_per_second: usize,
    ceiling_bytes_per_second: usize,
    burst_bytes: usize,
    queue_delay_target: Duration,
    feedback_timeout: Duration,
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
    last_feedback: Option<Instant>,
    rate_limited_since_feedback: bool,
    startup: bool,
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
    ) -> Arc<Self> {
        Arc::new(Self {
            floor_bytes_per_second,
            ceiling_bytes_per_second,
            burst_bytes,
            queue_delay_target,
            feedback_timeout,
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
    ) -> Arc<Self> {
        Arc::new(Self {
            floor_bytes_per_second: floor_bytes_per_second as f64,
            ceiling_bytes_per_second: ceiling_bytes_per_second as f64,
            burst_bytes: burst_bytes as f64,
            queue_delay_target,
            feedback_timeout,
            state: Mutex::new(TrafficState {
                tokens: burst_bytes as f64,
                last_refill: Instant::now(),
                current_bytes_per_second: floor_bytes_per_second as f64,
                minimum_round_trip: None,
                last_feedback: None,
                rate_limited_since_feedback: false,
                startup: true,
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
            state.prepare(
                Instant::now(),
                self.floor_bytes_per_second,
                self.burst_bytes,
                self.feedback_timeout,
            );
            if state.waiters.iter().all(|count| *count == 0)
                && state.has_tokens(bytes, self.burst_bytes)
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
                state.prepare(
                    Instant::now(),
                    self.floor_bytes_per_second,
                    self.burst_bytes,
                    self.feedback_timeout,
                );
                state.rate_limited_since_feedback = true;
                if state.can_send(priority, bytes, self.burst_bytes) {
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
                    self.burst_bytes,
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
    /// from persistently filling transport and network buffers above it: demand may double the
    /// rate after a low-delay sample, while one sample above the standing-queue target halves it.
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
            self.feedback_timeout,
        );
        let minimum = state
            .minimum_round_trip
            .map_or(round_trip, |minimum| minimum.min(round_trip));
        state.minimum_round_trip = Some(minimum);
        let queue_delay = round_trip.saturating_sub(minimum);
        let previous_rate = state.current_bytes_per_second;
        if queue_delay > self.queue_delay_target {
            state.current_bytes_per_second =
                (state.current_bytes_per_second * 0.5).max(self.floor_bytes_per_second);
            state.startup = false;
        } else if state.rate_limited_since_feedback && queue_delay <= self.queue_delay_target / 4 {
            let gain = if state.startup { 2.0 } else { 1.25 };
            state.current_bytes_per_second =
                (state.current_bytes_per_second * gain).min(self.ceiling_bytes_per_second);
        }
        state.last_feedback = Some(now);
        state.rate_limited_since_feedback = false;
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

    fn lock(&self) -> MutexGuard<'_, TrafficState> {
        match self.state.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
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
        feedback_timeout: Duration,
    ) {
        self.refill(now, self.current_bytes_per_second, burst_bytes);
        if self
            .last_feedback
            .is_some_and(|last| now.saturating_duration_since(last) > feedback_timeout)
        {
            self.current_bytes_per_second = floor_bytes_per_second;
            self.minimum_round_trip = None;
            self.last_feedback = None;
            self.rate_limited_since_feedback = false;
            self.startup = true;
        }
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
            last_feedback: None,
            rate_limited_since_feedback: false,
            startup: true,
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
            last_feedback: None,
            rate_limited_since_feedback: false,
            startup: true,
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
            last_feedback: None,
            rate_limited_since_feedback: false,
            startup: true,
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
            last_feedback: None,
            rate_limited_since_feedback: false,
            startup: true,
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
        );
        shaper.observe_round_trip(Duration::from_millis(40));
        assert_eq!(shaper.current_rate_bytes_per_second(), 100);
        shaper.lock().rate_limited_since_feedback = true;
        shaper.observe_round_trip(Duration::from_millis(42));
        assert_eq!(shaper.current_rate_bytes_per_second(), 200);
    }

    #[test]
    fn standing_queue_feedback_cuts_rate_to_the_floor() {
        let shaper = ClientTrafficShaper::new(
            100,
            800,
            100,
            Duration::from_millis(20),
            Duration::from_secs(3),
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
                shaper.feedback_timeout,
            );
        }
        let state = shaper.lock();
        assert_eq!(state.current_bytes_per_second, 100.0);
        assert_eq!(state.minimum_round_trip, None);
    }
}
