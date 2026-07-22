use voxels_world::WorldProductPriority;

/// Selects lower-value pending work that may be canceled to admit a more valuable request.
///
/// The browser and server negotiate a deliberately small request window. Letting old visible or
/// prefetch batches occupy every slot would invert the streaming priority before the server ever
/// sees the urgent request. Prefer the lowest-value pending class, then its newest request so work
/// that has had less time to complete is discarded first.
pub(crate) fn priority_preemption_candidate(
    incoming: WorldProductPriority,
    pending: impl IntoIterator<Item = (u64, WorldProductPriority)>,
) -> Option<u64> {
    pending
        .into_iter()
        .filter(|(_, priority)| *priority > incoming)
        .max_by_key(|(request_id, priority)| (*priority, *request_id))
        .map(|(request_id, _)| request_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collision_preempts_the_newest_lowest_value_request() {
        let pending = [
            (10, WorldProductPriority::VisibleChunk),
            (11, WorldProductPriority::Prefetch),
            (12, WorldProductPriority::CollisionCritical),
            (13, WorldProductPriority::Prefetch),
            (14, WorldProductPriority::VisibleSurface),
        ];

        assert_eq!(
            priority_preemption_candidate(WorldProductPriority::CollisionCritical, pending,),
            Some(13)
        );
    }

    #[test]
    fn visible_work_preempts_strictly_lower_value_work() {
        let pending = [
            (10, WorldProductPriority::Prefetch),
            (11, WorldProductPriority::VisibleSurface),
        ];

        assert_eq!(
            priority_preemption_candidate(WorldProductPriority::VisibleChunk, pending),
            Some(10)
        );
    }

    #[test]
    fn equal_priority_work_does_not_churn_the_window() {
        let pending = [
            (10, WorldProductPriority::VisibleSurface),
            (11, WorldProductPriority::VisibleSurface),
        ];

        assert_eq!(
            priority_preemption_candidate(WorldProductPriority::VisibleSurface, pending),
            None
        );
    }

    #[test]
    fn collision_never_displaces_collision() {
        let pending = [
            (10, WorldProductPriority::CollisionCritical),
            (11, WorldProductPriority::CollisionCritical),
        ];

        assert_eq!(
            priority_preemption_candidate(WorldProductPriority::CollisionCritical, pending,),
            None
        );
    }
}
