use voxels_world::WorldProductPriority;

/// Client request-window value order.
const fn request_window_rank(priority: WorldProductPriority) -> u8 {
    match priority {
        WorldProductPriority::CollisionCritical => 0,
        WorldProductPriority::VirtualTerrain => 1,
        WorldProductPriority::VisibleChunk => 2,
        WorldProductPriority::Prefetch => 3,
    }
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
    let incoming_rank = request_window_rank(incoming);
    pending
        .into_iter()
        .filter(|(_, priority)| request_window_rank(*priority) > incoming_rank)
        .max_by_key(|(request_id, priority)| (request_window_rank(*priority), *request_id))
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
            (14, WorldProductPriority::VisibleChunk),
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
            (11, WorldProductPriority::VisibleChunk),
        ];

        assert_eq!(
            priority_preemption_candidate(WorldProductPriority::VisibleChunk, pending),
            Some(10)
        );
    }

    #[test]
    fn virtual_terrain_preempts_visible_chunk_work() {
        let pending = [
            (10, WorldProductPriority::VisibleChunk),
            (11, WorldProductPriority::VirtualTerrain),
        ];

        assert_eq!(
            priority_preemption_candidate(WorldProductPriority::VirtualTerrain, pending),
            Some(10)
        );
    }

    #[test]
    fn virtual_terrain_preempts_visible_chunks_but_not_collision() {
        let pending = [
            (10, WorldProductPriority::VirtualTerrain),
            (11, WorldProductPriority::CollisionCritical),
            (12, WorldProductPriority::VisibleChunk),
        ];

        assert_eq!(
            priority_preemption_candidate(WorldProductPriority::VirtualTerrain, pending),
            Some(12)
        );
    }

    #[test]
    fn newest_visible_chunk_is_preempted_for_virtual_terrain() {
        let pending = [
            (10, WorldProductPriority::VisibleChunk),
            (11, WorldProductPriority::VirtualTerrain),
            (12, WorldProductPriority::VisibleChunk),
        ];

        assert_eq!(
            priority_preemption_candidate(WorldProductPriority::VirtualTerrain, pending),
            Some(12)
        );
    }

    #[test]
    fn visible_chunks_do_not_displace_virtual_terrain_work() {
        let pending = [
            (10, WorldProductPriority::VirtualTerrain),
            (11, WorldProductPriority::VisibleChunk),
        ];

        assert_eq!(
            priority_preemption_candidate(WorldProductPriority::VisibleChunk, pending),
            None
        );
    }

    #[test]
    fn equal_priority_work_does_not_churn_the_window() {
        let pending = [
            (10, WorldProductPriority::VisibleChunk),
            (11, WorldProductPriority::VisibleChunk),
        ];

        assert_eq!(
            priority_preemption_candidate(WorldProductPriority::VisibleChunk, pending),
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
