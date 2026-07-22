//! Priority-aware admission for process-wide blocking world generation.

use std::sync::{Arc, Mutex, MutexGuard};
use tokio::sync::Notify;
use voxels_world::WorldProductPriority;

/// Keeps the configured generation capacity while waking collision work first, then the bounded
/// current-surface ancestor chain, before queued ordinary generation. Already-executing work is
/// never interrupted.
pub(crate) struct PriorityGenerationLimiter {
    capacity: usize,
    state: Mutex<State>,
    collision_ready: Notify,
    immediate_ready: Notify,
    ordinary_ready: Notify,
}

#[derive(Default)]
struct State {
    available: usize,
    collision_waiters: usize,
    immediate_waiters: usize,
    ordinary_waiters: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum GenerationLane {
    Collision,
    Immediate,
    Ordinary,
}

impl GenerationLane {
    fn for_priority(priority: WorldProductPriority) -> Self {
        match priority {
            WorldProductPriority::CollisionCritical => Self::Collision,
            WorldProductPriority::ImmediateSurface => Self::Immediate,
            WorldProductPriority::VisibleChunk
            | WorldProductPriority::VisibleSurface
            | WorldProductPriority::ReplacementSurface
            | WorldProductPriority::Prefetch => Self::Ordinary,
        }
    }
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
            immediate_ready: Notify::new(),
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
        let wake_immediate =
            state.available > 0 && state.collision_waiters == 0 && state.immediate_waiters > 0;
        let wake_ordinary = state.available > 0
            && state.collision_waiters == 0
            && state.immediate_waiters == 0
            && state.ordinary_waiters > 0;
        drop(state);
        if wake_collision {
            self.collision_ready.notify_one();
        } else if wake_immediate {
            self.immediate_ready.notify_one();
        } else if wake_ordinary {
            self.ordinary_ready.notify_one();
        }
    }

    pub(crate) async fn acquire(
        self: &Arc<Self>,
        priority: WorldProductPriority,
    ) -> PriorityGenerationPermit {
        let lane = GenerationLane::for_priority(priority);
        let mut waiter = Waiter::new(Arc::clone(self), lane);
        loop {
            let ready = match lane {
                GenerationLane::Collision => self.collision_ready.notified(),
                GenerationLane::Immediate => self.immediate_ready.notified(),
                GenerationLane::Ordinary => self.ordinary_ready.notified(),
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
    pub(crate) fn waiting(&self) -> (usize, usize, usize) {
        let state = self.lock();
        (
            state.collision_waiters,
            state.immediate_waiters,
            state.ordinary_waiters,
        )
    }
}

struct Waiter {
    limiter: Arc<PriorityGenerationLimiter>,
    lane: GenerationLane,
    registered: bool,
}

impl Waiter {
    fn new(limiter: Arc<PriorityGenerationLimiter>, lane: GenerationLane) -> Self {
        {
            let mut state = limiter.lock();
            match lane {
                GenerationLane::Collision => state.collision_waiters += 1,
                GenerationLane::Immediate => state.immediate_waiters += 1,
                GenerationLane::Ordinary => state.ordinary_waiters += 1,
            }
        }
        Self {
            limiter,
            lane,
            registered: true,
        }
    }

    fn try_acquire(&mut self) -> bool {
        let mut state = self.limiter.lock();
        let eligible = state.available > 0
            && match self.lane {
                GenerationLane::Collision => true,
                GenerationLane::Immediate => state.collision_waiters == 0,
                GenerationLane::Ordinary => {
                    state.collision_waiters == 0 && state.immediate_waiters == 0
                }
            };
        if !eligible {
            return false;
        }
        state.available -= 1;
        match self.lane {
            GenerationLane::Collision => state.collision_waiters -= 1,
            GenerationLane::Immediate => state.immediate_waiters -= 1,
            GenerationLane::Ordinary => state.ordinary_waiters -= 1,
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
        match self.lane {
            GenerationLane::Collision => state.collision_waiters -= 1,
            GenerationLane::Immediate => state.immediate_waiters -= 1,
            GenerationLane::Ordinary => state.ordinary_waiters -= 1,
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
            while limiter.waiting().2 != 1 {
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

    #[tokio::test]
    async fn immediate_surface_generation_jumps_queued_ordinary_work() {
        let limiter = PriorityGenerationLimiter::new(1);
        let held = limiter.acquire(WorldProductPriority::Prefetch).await;
        let order = Arc::new(Mutex::new(Vec::new()));

        let ordinary_limiter = Arc::clone(&limiter);
        let ordinary_order = Arc::clone(&order);
        let ordinary = tokio::spawn(async move {
            let _permit = ordinary_limiter
                .acquire(WorldProductPriority::VisibleSurface)
                .await;
            ordinary_order
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push("ordinary");
        });
        tokio::time::timeout(Duration::from_secs(1), async {
            while limiter.waiting().2 != 1 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("ordinary waiter must register");

        let immediate_limiter = Arc::clone(&limiter);
        let immediate_order = Arc::clone(&order);
        let immediate = tokio::spawn(async move {
            let _permit = immediate_limiter
                .acquire(WorldProductPriority::ImmediateSurface)
                .await;
            immediate_order
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push("immediate");
        });
        tokio::time::timeout(Duration::from_secs(1), async {
            while limiter.waiting().1 != 1 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("immediate waiter must register");

        drop(held);
        tokio::time::timeout(Duration::from_secs(1), async {
            immediate.await.expect("immediate waiter");
            ordinary.await.expect("ordinary waiter");
        })
        .await
        .expect("generation waiters must complete");

        assert_eq!(
            *order
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
            ["immediate", "ordinary"]
        );
    }

    #[tokio::test]
    async fn collision_generation_still_jumps_immediate_surface_work() {
        let limiter = PriorityGenerationLimiter::new(1);
        let held = limiter.acquire(WorldProductPriority::Prefetch).await;
        let order = Arc::new(Mutex::new(Vec::new()));

        let immediate_limiter = Arc::clone(&limiter);
        let immediate_order = Arc::clone(&order);
        let immediate = tokio::spawn(async move {
            let _permit = immediate_limiter
                .acquire(WorldProductPriority::ImmediateSurface)
                .await;
            immediate_order
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push("immediate");
        });
        tokio::time::timeout(Duration::from_secs(1), async {
            while limiter.waiting().1 != 1 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("immediate waiter must register");

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
            immediate.await.expect("immediate waiter");
        })
        .await
        .expect("generation waiters must complete");

        assert_eq!(
            *order
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
            ["collision", "immediate"]
        );
    }
}
