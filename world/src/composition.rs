pub const COMPOSITION_EDGE_FEATURE_CELLS: i32 = 8;

#[repr(u8)]
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum FeatureCompositionMode {
    #[default]
    Cluster = 0,
    Ring = 1,
    Procession = 2,
    Clearing = 3,
}

impl FeatureCompositionMode {
    pub const ALL: [Self; 4] = [Self::Cluster, Self::Ring, Self::Procession, Self::Clearing];
}

#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct FeatureCompositionId {
    pub x: i32,
    pub z: i32,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct FeatureComposition {
    pub id: FeatureCompositionId,
    pub mode: FeatureCompositionMode,
    /// Focal feature-cell coordinates local to the 8x8 composition cell.
    pub focus: [i32; 2],
    pub orientation: u8,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct FeatureCompositionInfluence {
    /// Multiplier in [0, 255] applied to the biome's ordinary placement probability.
    pub density: u8,
    /// 0 = background, 1 = companion, 2 = unique hero landmark for the composition cell.
    pub prominence: u8,
}

impl FeatureComposition {
    pub fn for_feature_cell(feature_cell_x: i32, feature_cell_z: i32, hash: u64) -> Self {
        let id = FeatureCompositionId {
            x: feature_cell_x.div_euclid(COMPOSITION_EDGE_FEATURE_CELLS),
            z: feature_cell_z.div_euclid(COMPOSITION_EDGE_FEATURE_CELLS),
        };
        let mode = FeatureCompositionMode::ALL[(hash & 3) as usize];
        let focus = match mode {
            FeatureCompositionMode::Cluster => {
                [1 + ((hash >> 8) % 6) as i32, 1 + ((hash >> 16) % 6) as i32]
            }
            FeatureCompositionMode::Ring
            | FeatureCompositionMode::Procession
            | FeatureCompositionMode::Clearing => [3, 3],
        };
        Self {
            id,
            mode,
            focus,
            orientation: ((hash >> 24) & 3) as u8,
        }
    }

    pub fn influence(
        self,
        feature_cell_x: i32,
        feature_cell_z: i32,
    ) -> FeatureCompositionInfluence {
        let local = [
            feature_cell_x.rem_euclid(COMPOSITION_EDGE_FEATURE_CELLS),
            feature_cell_z.rem_euclid(COMPOSITION_EDGE_FEATURE_CELLS),
        ];
        let delta = [local[0] - self.focus[0], local[1] - self.focus[1]];
        let hero = self.hero_local_cell();
        if local == hero {
            return FeatureCompositionInfluence {
                density: u8::MAX,
                prominence: 2,
            };
        }
        match self.mode {
            FeatureCompositionMode::Cluster => match delta[0].abs().max(delta[1].abs()) {
                0 | 1 => FeatureCompositionInfluence {
                    density: 240,
                    prominence: 1,
                },
                2 => FeatureCompositionInfluence {
                    density: 150,
                    prominence: 0,
                },
                3 => FeatureCompositionInfluence {
                    density: 55,
                    prominence: 0,
                },
                _ => FeatureCompositionInfluence {
                    density: 15,
                    prominence: 0,
                },
            },
            FeatureCompositionMode::Ring => {
                let distance_squared = delta[0] * delta[0] + delta[1] * delta[1];
                if (4..=10).contains(&distance_squared) {
                    FeatureCompositionInfluence {
                        density: 220,
                        prominence: 1,
                    }
                } else if (2..=14).contains(&distance_squared) {
                    FeatureCompositionInfluence {
                        density: 90,
                        prominence: 0,
                    }
                } else {
                    FeatureCompositionInfluence {
                        density: 10,
                        prominence: 0,
                    }
                }
            }
            FeatureCompositionMode::Procession => {
                let perpendicular = match self.orientation & 3 {
                    0 => delta[1].abs(),
                    1 => delta[0].abs(),
                    2 => (delta[0] - delta[1]).abs(),
                    _ => (delta[0] + delta[1]).abs(),
                };
                match perpendicular {
                    0 => FeatureCompositionInfluence {
                        density: 220,
                        prominence: 1,
                    },
                    1 => FeatureCompositionInfluence {
                        density: 115,
                        prominence: 0,
                    },
                    _ => FeatureCompositionInfluence {
                        density: 10,
                        prominence: 0,
                    },
                }
            }
            FeatureCompositionMode::Clearing => {
                let distance = delta[0].abs().max(delta[1].abs());
                match distance {
                    0 | 1 => FeatureCompositionInfluence {
                        density: 0,
                        prominence: 0,
                    },
                    2 => FeatureCompositionInfluence {
                        density: 190,
                        prominence: 1,
                    },
                    _ => FeatureCompositionInfluence {
                        density: 60,
                        prominence: 0,
                    },
                }
            }
        }
    }

    pub fn hero_local_cell(self) -> [i32; 2] {
        let offset = match self.mode {
            FeatureCompositionMode::Cluster | FeatureCompositionMode::Procession => [0, 0],
            FeatureCompositionMode::Ring => rotate([2, 0], self.orientation),
            FeatureCompositionMode::Clearing => rotate([3, 0], self.orientation),
        };
        [self.focus[0] + offset[0], self.focus[1] + offset[1]]
    }
}

const fn rotate(offset: [i32; 2], orientation: u8) -> [i32; 2] {
    match orientation & 3 {
        0 => offset,
        1 => [-offset[1], offset[0]],
        2 => [-offset[0], -offset[1]],
        _ => [offset[1], -offset[0]],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn every_mode_has_one_bounded_hero_and_non_uniform_density() {
        for (mode_index, mode) in FeatureCompositionMode::ALL.into_iter().enumerate() {
            let composition = FeatureComposition::for_feature_cell(0, 0, mode_index as u64);
            assert_eq!(composition.mode, mode);
            let hero = composition.hero_local_cell();
            assert!((0..COMPOSITION_EDGE_FEATURE_CELLS).contains(&hero[0]));
            assert!((0..COMPOSITION_EDGE_FEATURE_CELLS).contains(&hero[1]));
            let mut hero_count = 0;
            let mut densities = BTreeSet::new();
            for z in 0..COMPOSITION_EDGE_FEATURE_CELLS {
                for x in 0..COMPOSITION_EDGE_FEATURE_CELLS {
                    let influence = composition.influence(x, z);
                    densities.insert(influence.density);
                    hero_count += usize::from(influence.prominence == 2);
                }
            }
            assert_eq!(hero_count, 1);
            assert!(densities.len() >= 3);
        }
    }

    #[test]
    fn negative_feature_cells_share_their_euclidean_composition_identity() {
        let composition = FeatureComposition::for_feature_cell(-1, -1, 0x1234);
        assert_eq!(composition.id, FeatureCompositionId { x: -1, z: -1 });
        for z in -8..0 {
            for x in -8..0 {
                let candidate = FeatureComposition::for_feature_cell(x, z, 0x1234);
                assert_eq!(candidate.id, composition.id);
                assert_eq!(candidate.influence(x, z), composition.influence(x, z));
            }
        }
    }

    #[test]
    fn processions_cross_the_composition_cell_in_every_orientation() {
        for orientation in 0..4 {
            let hash = (orientation as u64) << 24 | FeatureCompositionMode::Procession as u64;
            let composition = FeatureComposition::for_feature_cell(0, 0, hash);
            let companions = (0..COMPOSITION_EDGE_FEATURE_CELLS)
                .flat_map(|z| (0..COMPOSITION_EDGE_FEATURE_CELLS).map(move |x| [x, z]))
                .filter(|cell| composition.influence(cell[0], cell[1]).prominence > 0)
                .collect::<Vec<_>>();
            let boundary_sides = [
                companions.iter().any(|cell| cell[0] == 0),
                companions
                    .iter()
                    .any(|cell| cell[0] == COMPOSITION_EDGE_FEATURE_CELLS - 1),
                companions.iter().any(|cell| cell[1] == 0),
                companions
                    .iter()
                    .any(|cell| cell[1] == COMPOSITION_EDGE_FEATURE_CELLS - 1),
            ];
            assert!(
                boundary_sides
                    .into_iter()
                    .filter(|touched| *touched)
                    .count()
                    >= 2
            );
        }
    }
}
