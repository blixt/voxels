use voxels_world::WorldProductPriority;

/// Whether an in-flight batch should yield its request slot after the streaming focus changes.
///
/// The completion path requeues tickets that still belong to the new focus and drops obsolete
/// tickets. Canceling a partly stale batch is therefore both safe and necessary: otherwise one
/// barely relevant tile can let three obsolete siblings hold an equal-priority socket slot.
pub(crate) fn batch_has_obsolete_item(relevance: impl IntoIterator<Item = bool>) -> bool {
    relevance.into_iter().any(|relevant| !relevant)
}

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
    fn immediate_surface_preempts_older_visible_surface_work() {
        let pending = [
            (10, WorldProductPriority::VisibleSurface),
            (11, WorldProductPriority::ImmediateSurface),
        ];

        assert_eq!(
            priority_preemption_candidate(WorldProductPriority::ImmediateSurface, pending),
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

    #[test]
    fn a_partly_obsolete_batch_yields_its_request_slot() {
        assert!(!batch_has_obsolete_item([true, true, true, true]));
        assert!(batch_has_obsolete_item([true, false, true, true]));
        assert!(batch_has_obsolete_item([false, false]));
    }
}
