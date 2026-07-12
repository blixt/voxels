//! Small allocation-free visibility graphs for authored caves and interior spaces.
//!
//! The graph is derived world metadata: durable voxels remain authoritative, while portal state can
//! be recomputed from the generator plus sparse edits. Fixed capacities keep queries predictable in
//! browser/WASM and native hosts.

pub const MAX_VISIBILITY_CELLS: usize = 16;
pub const MAX_VISIBILITY_PORTALS: usize = 32;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct VisibilityCellId(u8);

impl VisibilityCellId {
    pub const fn new(index: u8) -> Self {
        Self(index)
    }

    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VisibilityPortal {
    pub from: VisibilityCellId,
    pub to: VisibilityCellId,
    pub distance_metres: f32,
}

impl VisibilityPortal {
    pub const fn new(from: VisibilityCellId, to: VisibilityCellId, distance_metres: f32) -> Self {
        Self {
            from,
            to,
            distance_metres,
        }
    }
}

const EMPTY_PORTAL: VisibilityPortal =
    VisibilityPortal::new(VisibilityCellId::new(0), VisibilityCellId::new(0), 0.0);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VisibilityGraphError {
    NoCells,
    TooManyCells,
    TooManyPortals,
    InvalidPortal,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PortalState {
    open_bits: u32,
}

impl PortalState {
    pub const fn all_open(portal_count: usize) -> Self {
        let open_bits = if portal_count >= u32::BITS as usize {
            u32::MAX
        } else if portal_count == 0 {
            0
        } else {
            (1u32 << portal_count) - 1
        };
        Self { open_bits }
    }

    pub const fn is_open(self, portal_index: usize) -> bool {
        portal_index < MAX_VISIBILITY_PORTALS && self.open_bits & (1u32 << portal_index) != 0
    }

    pub fn set_open(&mut self, portal_index: usize, open: bool) -> bool {
        if portal_index >= MAX_VISIBILITY_PORTALS {
            return false;
        }
        let mask = 1u32 << portal_index;
        if open {
            self.open_bits |= mask;
        } else {
            self.open_bits &= !mask;
        }
        true
    }
}

#[derive(Clone, Copy, Debug)]
pub struct VisibilityGraph {
    cell_count: u8,
    portal_count: u8,
    portals: [VisibilityPortal; MAX_VISIBILITY_PORTALS],
}

impl VisibilityGraph {
    pub fn new(
        cell_count: usize,
        portals: &[VisibilityPortal],
    ) -> Result<Self, VisibilityGraphError> {
        if cell_count == 0 {
            return Err(VisibilityGraphError::NoCells);
        }
        if cell_count > MAX_VISIBILITY_CELLS {
            return Err(VisibilityGraphError::TooManyCells);
        }
        if portals.len() > MAX_VISIBILITY_PORTALS {
            return Err(VisibilityGraphError::TooManyPortals);
        }
        if portals.iter().any(|portal| {
            portal.from.index() >= cell_count
                || portal.to.index() >= cell_count
                || portal.from == portal.to
                || !portal.distance_metres.is_finite()
                || portal.distance_metres < 0.0
        }) {
            return Err(VisibilityGraphError::InvalidPortal);
        }
        let mut stored = [EMPTY_PORTAL; MAX_VISIBILITY_PORTALS];
        stored[..portals.len()].copy_from_slice(portals);
        Ok(Self {
            cell_count: cell_count as u8,
            portal_count: portals.len() as u8,
            portals: stored,
        })
    }

    pub const fn cell_count(&self) -> usize {
        self.cell_count as usize
    }

    pub const fn portal_count(&self) -> usize {
        self.portal_count as usize
    }

    pub fn shortest_open_distance(
        &self,
        from: VisibilityCellId,
        to: VisibilityCellId,
        portal_state: PortalState,
    ) -> Option<f32> {
        if from.index() >= self.cell_count() || to.index() >= self.cell_count() {
            return None;
        }
        if from == to {
            return Some(0.0);
        }

        let mut distance = [f32::INFINITY; MAX_VISIBILITY_CELLS];
        let mut visited = [false; MAX_VISIBILITY_CELLS];
        distance[from.index()] = 0.0;

        for _ in 0..self.cell_count() {
            let current = (0..self.cell_count())
                .filter(|index| !visited[*index])
                .min_by(|left, right| {
                    distance[*left]
                        .total_cmp(&distance[*right])
                        .then_with(|| left.cmp(right))
                })?;
            if !distance[current].is_finite() {
                break;
            }
            if current == to.index() {
                return Some(distance[current]);
            }
            visited[current] = true;

            for (portal_index, portal) in self.portals[..self.portal_count()].iter().enumerate() {
                if !portal_state.is_open(portal_index) {
                    continue;
                }
                let neighbor = if portal.from.index() == current {
                    Some(portal.to.index())
                } else if portal.to.index() == current {
                    Some(portal.from.index())
                } else {
                    None
                };
                let Some(neighbor) = neighbor else {
                    continue;
                };
                let candidate = distance[current] + portal.distance_metres;
                if candidate < distance[neighbor] {
                    distance[neighbor] = candidate;
                }
            }
        }
        distance[to.index()]
            .is_finite()
            .then_some(distance[to.index()])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const fn cell(index: u8) -> VisibilityCellId {
        VisibilityCellId::new(index)
    }

    #[test]
    fn shortest_path_is_deterministic_and_uses_geodesic_distance() {
        let graph = VisibilityGraph::new(
            4,
            &[
                VisibilityPortal::new(cell(0), cell(1), 4.0),
                VisibilityPortal::new(cell(1), cell(2), 5.0),
                VisibilityPortal::new(cell(0), cell(3), 1.0),
                VisibilityPortal::new(cell(3), cell(2), 20.0),
            ],
        )
        .unwrap();
        let state = PortalState::all_open(graph.portal_count());
        assert_eq!(
            graph.shortest_open_distance(cell(0), cell(2), state),
            Some(9.0)
        );
        assert_eq!(
            graph.shortest_open_distance(cell(2), cell(0), state),
            Some(9.0)
        );
        assert_eq!(
            graph.shortest_open_distance(cell(2), cell(2), state),
            Some(0.0)
        );
    }

    #[test]
    fn closing_one_portal_disconnects_the_downstream_cell() {
        let graph = VisibilityGraph::new(
            3,
            &[
                VisibilityPortal::new(cell(0), cell(1), 1.0),
                VisibilityPortal::new(cell(1), cell(2), 1.0),
            ],
        )
        .unwrap();
        let mut state = PortalState::all_open(graph.portal_count());
        assert_eq!(
            graph.shortest_open_distance(cell(0), cell(2), state),
            Some(2.0)
        );
        assert!(state.set_open(1, false));
        assert_eq!(graph.shortest_open_distance(cell(0), cell(2), state), None);
        assert!(state.set_open(1, true));
        assert_eq!(
            graph.shortest_open_distance(cell(0), cell(2), state),
            Some(2.0)
        );
    }

    #[test]
    fn capacities_and_invalid_definitions_are_rejected() {
        assert_eq!(
            VisibilityGraph::new(0, &[]).unwrap_err(),
            VisibilityGraphError::NoCells
        );
        assert_eq!(
            VisibilityGraph::new(MAX_VISIBILITY_CELLS + 1, &[]).unwrap_err(),
            VisibilityGraphError::TooManyCells
        );
        let too_many = vec![VisibilityPortal::new(cell(0), cell(1), 1.0); 33];
        assert_eq!(
            VisibilityGraph::new(2, &too_many).unwrap_err(),
            VisibilityGraphError::TooManyPortals
        );
        assert_eq!(
            VisibilityGraph::new(2, &[VisibilityPortal::new(cell(0), cell(2), 1.0)]).unwrap_err(),
            VisibilityGraphError::InvalidPortal
        );
        assert_eq!(
            VisibilityGraph::new(2, &[VisibilityPortal::new(cell(0), cell(1), f32::NAN)])
                .unwrap_err(),
            VisibilityGraphError::InvalidPortal
        );
    }

    #[test]
    fn equal_cost_routes_have_stable_cell_order() {
        let graph = VisibilityGraph::new(
            4,
            &[
                VisibilityPortal::new(cell(0), cell(2), 1.0),
                VisibilityPortal::new(cell(2), cell(3), 1.0),
                VisibilityPortal::new(cell(0), cell(1), 1.0),
                VisibilityPortal::new(cell(1), cell(3), 1.0),
            ],
        )
        .unwrap();
        let state = PortalState::all_open(graph.portal_count());
        for _ in 0..16 {
            assert_eq!(
                graph.shortest_open_distance(cell(0), cell(3), state),
                Some(2.0)
            );
        }
    }
}
