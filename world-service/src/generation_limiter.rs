//! Priority-aware admission for process-wide blocking world generation.

use std::sync::{Arc, Mutex, MutexGuard};
use tokio::sync::Notify;
use voxels_world::WorldProductPriority;

/// Keeps the configured process-wide capacity while waking collision work before queued ordinary
/// generation. Already-executing work is never interrupted.
pub(crate) struct PriorityGenerationLimiter {
    capacity: usize,
    state: Mutex<State>,
    collision_ready: Notify,
    ordinary_ready: Notify,
}

#[derive(Default)]
struct State {
    available: usize,
    collision_waiters: usize,
    ordinary_waiters: usize,
}

impl PriorityGenerationLimiter {
    pub(crate) fn new(capacity: usize) -> Arc<Self> {
        assert!(capacity > 0, "generation capacity must be nonzero");
        Arc::new(Self {
            capacity,
            state: Mutex::new(State {
                available: capacity,
                ..State::default()
            }),
            collision_ready: Notify::new(),
            ordinary_ready: Notify::new(),
        })
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        match self.state.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    fn signal(&self) {
        let state = self.lock();
        let wake_collision = state.available > 0 && state.collision_waiters > 0;
        let wake_ordinary =
            state.available > 0 && state.collision_waiters == 0 && state.ordinary_waiters > 0;
        drop(state);
        if wake_collision {
            self.collision_ready.notify_one();
        } else if wake_ordinary {
            self.ordinary_ready.notify_one();
        }
    }

    pub(crate) async fn acquire(
        self: &Arc<Self>,
        priority: WorldProductPriority,
    ) -> PriorityGenerationPermit {
        let collision = priority == WorldProductPriority::CollisionCritical;
        let mut waiter = Waiter::new(Arc::clone(self), collision);
        loop {
            let ready = if collision {
                self.collision_ready.notified()
            } else {
                self.ordinary_ready.notified()
            };
            tokio::pin!(ready);
            // Register with Notify before inspecting capacity so a release cannot be lost between
            // the state check and await.
            ready.as_mut().enable();
            if waiter.try_acquire() {
                return PriorityGenerationPermit {
                    limiter: Arc::clone(self),
                };
            }
            ready.await;
        }
    }

    #[cfg(test)]
    fn waiting(&self) -> (usize, usize) {
        let state = self.lock();
        (state.collision_waiters, state.ordinary_waiters)
    }
}

struct Waiter {
    limiter: Arc<PriorityGenerationLimiter>,
    collision: bool,
    registered: bool,
}

impl Waiter {
    fn new(limiter: Arc<PriorityGenerationLimiter>, collision: bool) -> Self {
        {
            let mut state = limiter.lock();
            if collision {
                state.collision_waiters += 1;
            } else {
                state.ordinary_waiters += 1;
            }
        }
        Self {
            limiter,
            collision,
            registered: true,
        }
    }

    fn try_acquire(&mut self) -> bool {
        let mut state = self.limiter.lock();
        let eligible = state.available > 0 && (self.collision || state.collision_waiters == 0);
        if !eligible {
            return false;
        }
        state.available -= 1;
        if self.collision {
            state.collision_waiters -= 1;
        } else {
            state.ordinary_waiters -= 1;
        }
        self.registered = false;
        drop(state);
        self.limiter.signal();
        true
    }
}

impl Drop for Waiter {
    fn drop(&mut self) {
        if !self.registered {
            return;
        }
        let mut state = self.limiter.lock();
        if self.collision {
            state.collision_waiters -= 1;
        } else {
            state.ordinary_waiters -= 1;
        }
        drop(state);
        self.limiter.signal();
    }
}

pub(crate) struct PriorityGenerationPermit {
    limiter: Arc<PriorityGenerationLimiter>,
}

impl Drop for PriorityGenerationPermit {
    fn drop(&mut self) {
        let mut state = self.limiter.lock();
        state.available += 1;
        debug_assert!(state.available <= self.limiter.capacity);
        drop(state);
        self.limiter.signal();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn collision_generation_jumps_queued_ordinary_work() {
        let limiter = PriorityGenerationLimiter::new(1);
        let held = limiter.acquire(WorldProductPriority::VisibleSurface).await;
        let order = Arc::new(Mutex::new(Vec::new()));

        let ordinary_limiter = Arc::clone(&limiter);
        let ordinary_order = Arc::clone(&order);
        let ordinary = tokio::spawn(async move {
            let _permit = ordinary_limiter
                .acquire(WorldProductPriority::VisibleChunk)
                .await;
            ordinary_order
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push("ordinary");
        });
        tokio::time::timeout(Duration::from_secs(1), async {
            while limiter.waiting().1 != 1 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("ordinary waiter must register");

        let collision_limiter = Arc::clone(&limiter);
        let collision_order = Arc::clone(&order);
        let collision = tokio::spawn(async move {
            let _permit = collision_limiter
                .acquire(WorldProductPriority::CollisionCritical)
                .await;
            collision_order
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push("collision");
        });
        tokio::time::timeout(Duration::from_secs(1), async {
            while limiter.waiting().0 != 1 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("collision waiter must register");

        drop(held);
        tokio::time::timeout(Duration::from_secs(1), async {
            collision.await.expect("collision waiter");
            ordinary.await.expect("ordinary waiter");
        })
        .await
        .expect("generation waiters must complete");

        assert_eq!(
            *order
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
            ["collision", "ordinary"]
        );
    }
}
