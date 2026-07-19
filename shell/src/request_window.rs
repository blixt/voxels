use voxels_world::WorldProductPriority;

/// Selects an ordinary request that may be canceled to admit new collision-critical work.
///
/// The browser and server negotiate a deliberately small request window. Letting old visible or
/// prefetch batches occupy every slot would invert the streaming priority before the server ever
/// sees the urgent request. Prefer the lowest-value pending class, then its newest request so work
/// that has had less time to complete is discarded first.
pub(crate) fn collision_preemption_candidate(
    incoming: WorldProductPriority,
    pending: impl IntoIterator<Item = (u64, WorldProductPriority)>,
) -> Option<u64> {
    if incoming != WorldProductPriority::CollisionCritical {
        return None;
    }
    pending
        .into_iter()
        .filter(|(_, priority)| *priority != WorldProductPriority::CollisionCritical)
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
            collision_preemption_candidate(WorldProductPriority::CollisionCritical, pending,),
            Some(13)
        );
    }

    #[test]
    fn ordinary_work_never_preempts_an_existing_request() {
        let pending = [
            (10, WorldProductPriority::Prefetch),
            (11, WorldProductPriority::VisibleSurface),
        ];

        assert_eq!(
            collision_preemption_candidate(WorldProductPriority::VisibleChunk, pending),
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
            collision_preemption_candidate(WorldProductPriority::CollisionCritical, pending,),
            None
        );
    }
}
