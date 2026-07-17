use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use tokio::sync::Notify;
use tokio::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum TrafficPriority {
    Critical = 0,
    Interactive = 1,
    VisibleWorld = 2,
    BackgroundWorld = 3,
}

impl TrafficPriority {
    pub(crate) const COUNT: usize = 4;
    const WEIGHTS: [f64; Self::COUNT] = [f64::INFINITY, 7.0, 3.0, 0.5];

    pub(crate) const fn index(self) -> usize {
        self as usize
    }
}

pub(crate) struct ClientTrafficShaper {
    bytes_per_second: f64,
    burst_bytes: f64,
    state: Mutex<TrafficState>,
    changed: Notify,
}

pub(crate) struct ClientTrafficRegistry {
    bytes_per_second: usize,
    burst_bytes: usize,
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
    waiters: [usize; TrafficPriority::COUNT],
    virtual_service: [f64; TrafficPriority::COUNT],
}

impl ClientTrafficRegistry {
    pub(crate) fn new(bytes_per_second: usize, burst_bytes: usize) -> Arc<Self> {
        Arc::new(Self {
            bytes_per_second,
            burst_bytes,
            clients: Mutex::new(HashMap::new()),
        })
    }

    pub(crate) fn register(self: &Arc<Self>, connection_id: u64) -> ClientTrafficRegistration {
        let shaper = ClientTrafficShaper::new(self.bytes_per_second, self.burst_bytes);
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
    pub(crate) fn new(bytes_per_second: usize, burst_bytes: usize) -> Arc<Self> {
        Arc::new(Self {
            bytes_per_second: bytes_per_second as f64,
            burst_bytes: burst_bytes as f64,
            state: Mutex::new(TrafficState {
                tokens: burst_bytes as f64,
                last_refill: Instant::now(),
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
            state.refill(Instant::now(), self.bytes_per_second, self.burst_bytes);
            if state.waiters.iter().all(|count| *count == 0)
                && state.has_tokens(bytes, self.burst_bytes)
            {
                state.consume(priority, bytes);
                return;
            }
        }
        let mut waiter = TrafficWaiter::new(Arc::clone(self), priority);
        loop {
            let notified = self.changed.notified();
            let delay = {
                let mut state = self.lock();
                state.refill(Instant::now(), self.bytes_per_second, self.burst_bytes);
                if state.can_send(priority, bytes, self.burst_bytes) {
                    state.consume(priority, bytes);
                    waiter.complete(&mut state);
                    if state.waiters.iter().any(|count| *count > 0) {
                        self.changed.notify_waiters();
                    }
                    return;
                }
                state.delay(priority, bytes, self.bytes_per_second, self.burst_bytes)
            };
            tokio::select! {
                () = tokio::time::sleep(delay) => {}
                () = notified => {}
            }
        }
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
            TrafficPriority::Interactive,
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
            waiters: [1, 0, 0, 0],
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
            waiters: [1, 0, 0, 1],
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
            waiters: [0, 1, 1, 1],
            virtual_service: [0.0; TrafficPriority::COUNT],
        };
        assert_eq!(
            state.selected_weighted_priority(),
            Some(TrafficPriority::Interactive)
        );
        state.consume(TrafficPriority::Interactive, 700);
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
            Some(TrafficPriority::Interactive)
        );
    }

    #[test]
    fn refill_never_exceeds_burst() {
        let now = Instant::now();
        let mut state = TrafficState {
            tokens: 20.0,
            last_refill: now,
            waiters: [0; TrafficPriority::COUNT],
            virtual_service: [0.0; TrafficPriority::COUNT],
        };
        state.refill(now + Duration::from_secs(10), 100.0, 200.0);
        assert_eq!(state.tokens, 200.0);
    }
}
