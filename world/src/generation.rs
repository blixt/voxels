use crate::{
    CHUNK_EDGE, COMPOSITION_EDGE_FEATURE_CELLS, Chunk, ChunkCoord, FEATURE_CELL_VOXELS,
    FEATURE_MAX_RADIUS_VOXELS, FIRST_PILGRIM_ROAD_NODES, FeatureComposition, Material,
    ROUTE_CORE_HALF_WIDTH_VOXELS, RouteAnchor, RouteAnchorRole, RouteLandmarkId, RouteSample,
    SkylineFeature, SkylineFeatureId, SkylineFeatureKind,
    first_pilgrim_route_anchor_for_feature_cell, sample_first_pilgrim_road,
};

/// Generator version is part of world identity. Changing terrain semantics requires incrementing it.
pub const GENERATOR_VERSION: u32 = 10;
pub const SEA_LEVEL_VOXELS: i32 = 10;

/// Renderer-neutral continuous atmosphere inputs derived from the same regional weights as terrain.
/// Every component is normalized to `[0, 1]`; shells may map them to lighting and weather without
/// duplicating world-generation classification.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AtmosphereSample {
    pub humidity: f32,
    pub coldness: f32,
    pub aerosol: f32,
    pub cloudiness: f32,
    pub horizon_warmth: f32,
    pub haze: f32,
}

impl AtmosphereSample {
    pub const fn components(self) -> [f32; 6] {
        [
            self.humidity,
            self.coldness,
            self.aerosol,
            self.cloudiness,
            self.horizon_warmth,
            self.haze,
        ]
    }

    pub fn is_finite_and_normalized(self) -> bool {
        self.components()
            .into_iter()
            .all(|value| value.is_finite() && (0.0..=1.0).contains(&value))
    }

    pub fn lerp(self, target: Self, amount: f32) -> Self {
        let amount = amount.clamp(0.0, 1.0);
        let blend = |from: f32, to: f32| from + (to - from) * amount;
        Self {
            humidity: blend(self.humidity, target.humidity),
            coldness: blend(self.coldness, target.coldness),
            aerosol: blend(self.aerosol, target.aerosol),
            cloudiness: blend(self.cloudiness, target.cloudiness),
            horizon_warmth: blend(self.horizon_warmth, target.horizon_warmth),
            haze: blend(self.haze, target.haze),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[repr(u8)]
pub enum SurfaceRegion {
    VerdantForest,
    WindMoor,
    Alpine,
    RedBadlands,
    PaleDunes,
    Volcanic,
}

impl SurfaceRegion {
    pub const ALL: [Self; 6] = [
        Self::VerdantForest,
        Self::WindMoor,
        Self::Alpine,
        Self::RedBadlands,
        Self::PaleDunes,
        Self::Volcanic,
    ];
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SurfaceSample {
    pub height: i32,
    pub material: Material,
    pub water_level: Option<i32>,
    pub region: SurfaceRegion,
    pub moisture: f32,
    pub temperature: f32,
    pub ridge: f32,
    pub atmosphere: AtmosphereSample,
    pub route: Option<RouteSample>,
}

#[derive(Clone, Copy, Debug)]
pub struct Generator {
    seed: u64,
}

#[derive(Clone, Copy, Debug)]
struct ColumnProfile {
    height: i32,
    moisture: f32,
    temperature: f32,
    ridge: f32,
    region: SurfaceRegion,
    material: Material,
    atmosphere: AtmosphereSample,
    route: Option<RouteSample>,
}

/// Reusable authoritative sampler for one X/Z column. Meshing a halo touches many Y values at the
/// same horizontal coordinate, so retaining its regional fields and feature intersections avoids
/// recomputing the full climate stack for every voxel.
#[derive(Clone, Debug)]
pub struct GeneratedColumn {
    generator: Generator,
    x: i32,
    z: i32,
    profile: ColumnProfile,
    features: [SkylineFeature; 9],
    feature_count: u8,
}

/// Reusable authoritative sampler for a rectangular X/Z region. Analytic features are discovered
/// once for the whole region instead of once for every column, which keeps chunk halo construction
/// deterministic without repeating placement work.
#[derive(Clone, Debug)]
pub struct GeneratedRegion {
    min_x: i32,
    min_z: i32,
    width: usize,
    depth: usize,
    columns: Vec<GeneratedColumn>,
}

impl GeneratedRegion {
    pub fn sample(&self, x: i32, y: i32, z: i32) -> Material {
        assert!(x >= self.min_x && z >= self.min_z);
        let local_x = (i64::from(x) - i64::from(self.min_x)) as usize;
        let local_z = (i64::from(z) - i64::from(self.min_z)) as usize;
        assert!(local_x < self.width && local_z < self.depth);
        self.columns[local_x + local_z * self.width].sample(y)
    }
}

impl GeneratedColumn {
    pub fn sample(&self, y: i32) -> Material {
        let terrain = self
            .generator
            .sample_terrain_with_profile(self.x, y, self.z, self.profile);
        if terrain.is_collidable() {
            return terrain;
        }
        self.features[..usize::from(self.feature_count)]
            .iter()
            .find_map(|feature| feature.material_at(crate::VoxelCoord::new(self.x, y, self.z)))
            .unwrap_or(terrain)
    }
}

impl Generator {
    pub const fn new(seed: u64) -> Self {
        Self { seed }
    }

    pub const fn seed(self) -> u64 {
        self.seed
    }

    pub fn generate_chunk(self, coord: ChunkCoord) -> Chunk {
        let mut chunk = Chunk::empty(coord);
        let origin = coord.world_origin();
        let columns = (0..CHUNK_EDGE)
            .flat_map(|z| {
                (0..CHUNK_EDGE)
                    .map(move |x| self.column_profile(origin[0] + x as i32, origin[2] + z as i32))
            })
            .collect::<Vec<_>>();
        for y in 0..CHUNK_EDGE {
            for z in 0..CHUNK_EDGE {
                for x in 0..CHUNK_EDGE {
                    let world = [
                        origin[0] + x as i32,
                        origin[1] + y as i32,
                        origin[2] + z as i32,
                    ];
                    chunk.set(
                        x,
                        y,
                        z,
                        self.sample_terrain_with_profile(
                            world[0],
                            world[1],
                            world[2],
                            columns[x + z * CHUNK_EDGE],
                        ),
                    );
                }
            }
        }
        self.decorate_features(&mut chunk);
        chunk
    }

    pub fn sample(self, x: i32, y: i32, z: i32) -> Material {
        self.column(x, z).sample(y)
    }

    pub fn column(self, x: i32, z: i32) -> GeneratedColumn {
        let mut features = [SkylineFeature::default(); 9];
        let mut feature_count = 0u8;
        let cell_x = x.div_euclid(FEATURE_CELL_VOXELS);
        let cell_z = z.div_euclid(FEATURE_CELL_VOXELS);
        for dz_cell in -1..=1 {
            for dx_cell in -1..=1 {
                let candidate_x = cell_x + dx_cell;
                let candidate_z = cell_z + dz_cell;
                let Some(feature) = self.skyline_feature(candidate_x, candidate_z) else {
                    continue;
                };
                if !feature.contains_xz(x, z) {
                    continue;
                }
                debug_assert!(usize::from(feature_count) < features.len());
                features[usize::from(feature_count)] = feature;
                feature_count += 1;
            }
        }
        GeneratedColumn {
            generator: self,
            x,
            z,
            profile: self.column_profile(x, z),
            features,
            feature_count,
        }
    }

    pub fn region(self, min_x: i32, min_z: i32, width: usize, depth: usize) -> GeneratedRegion {
        let max_x =
            min_x.saturating_add(i32::try_from(width.saturating_sub(1)).unwrap_or(i32::MAX));
        let max_z =
            min_z.saturating_add(i32::try_from(depth.saturating_sub(1)).unwrap_or(i32::MAX));
        let min_cell_x = min_x.div_euclid(FEATURE_CELL_VOXELS) - 1;
        let max_cell_x = max_x.div_euclid(FEATURE_CELL_VOXELS) + 1;
        let min_cell_z = min_z.div_euclid(FEATURE_CELL_VOXELS) - 1;
        let max_cell_z = max_z.div_euclid(FEATURE_CELL_VOXELS) + 1;
        let mut features = Vec::new();
        for cell_z in min_cell_z..=max_cell_z {
            for cell_x in min_cell_x..=max_cell_x {
                if let Some(feature) = self.skyline_feature(cell_x, cell_z) {
                    features.push(feature);
                }
            }
        }

        let mut columns = Vec::with_capacity(width.saturating_mul(depth));
        for local_z in 0..depth {
            for local_x in 0..width {
                let x = min_x.saturating_add(i32::try_from(local_x).unwrap_or(i32::MAX));
                let z = min_z.saturating_add(i32::try_from(local_z).unwrap_or(i32::MAX));
                let mut column_features = [SkylineFeature::default(); 9];
                let mut feature_count = 0u8;
                for feature in features
                    .iter()
                    .copied()
                    .filter(|feature| feature.contains_xz(x, z))
                {
                    debug_assert!(usize::from(feature_count) < column_features.len());
                    column_features[usize::from(feature_count)] = feature;
                    feature_count += 1;
                }
                columns.push(GeneratedColumn {
                    generator: self,
                    x,
                    z,
                    profile: self.column_profile(x, z),
                    features: column_features,
                    feature_count,
                });
            }
        }
        GeneratedRegion {
            min_x,
            min_z,
            width,
            depth,
            columns,
        }
    }

    fn sample_terrain_with_profile(
        self,
        x: i32,
        y: i32,
        z: i32,
        profile: ColumnProfile,
    ) -> Material {
        if y < -16 {
            return Material::Basalt;
        }
        let height = profile.height;
        if y > height {
            return if height < SEA_LEVEL_VOXELS && y <= SEA_LEVEL_VOXELS {
                Material::Water
            } else {
                Material::Air
            };
        }

        // Broad caves only affect material well under the surface, leaving a stable walking crust.
        if y + 4 < height {
            let cave = self.value_3d(x, y, z, 90, 0x9e37);
            let tunnel = self.value_3d(x, y * 2, z, 150, 0xb529);
            if cave > 0.73 && tunnel > 0.43 {
                return Material::Air;
            }
        }

        let depth = height - y;
        if depth == 0 {
            profile.material
        } else if depth < 4 {
            match profile.material {
                Material::Sand => Material::Sand,
                Material::RedSand => {
                    if depth == 1 {
                        Material::RedSand
                    } else {
                        Material::Clay
                    }
                }
                Material::Clay => Material::Clay,
                Material::Basalt => Material::Basalt,
                Material::Limestone => Material::Limestone,
                Material::Stone | Material::Snow => Material::Stone,
                _ => Material::Dirt,
            }
        } else if y < 2 && self.value_3d(x, y, z, 40, 0x55ad) > 0.66 {
            Material::Basalt
        } else {
            Material::Stone
        }
    }

    fn column_profile(self, x: i32, z: i32) -> ColumnProfile {
        let natural = self.natural_column_profile(x, z);
        self.apply_route_profile(x, z, natural)
    }

    fn natural_column_profile(self, x: i32, z: i32) -> ColumnProfile {
        let moisture = self.value_2d(x, z, 1_200, 0x4f1b);
        let temperature = self.value_2d(x, z, 1_900, 0xa18d);
        let local_biome = self.value_2d(x, z, 260, 0x8b21);
        let continental = self.fractal_2d(x, z, 1_800, 4, 0x71a9);
        let hills = self.fractal_2d(x, z, 420, 3, 0x2d31);
        let detail = self.fractal_2d(x, z, 64, 2, 0x51f7);
        let ridge = 1.0 - (self.value_2d(x, z, 620, 0xc43b) * 2.0 - 1.0).abs();
        let volcanic_field = self.fractal_2d(x, z, 980, 3, 0x6d2b);
        let volcanic_core = smooth(((volcanic_field - 0.62) / 0.38).clamp(0.0, 1.0));

        let weights = [
            moisture * (1.0 - (temperature - 0.52).abs()) * (0.65 + local_biome * 0.35),
            (1.0 - (moisture - 0.48).abs()) * (1.15 - temperature) * (1.0 - ridge * 0.35),
            ridge * ridge * (0.62 + continental * 0.52) * (1.18 - temperature * 0.42),
            (1.0 - moisture) * temperature * (0.42 + local_biome * 0.90),
            (1.0 - moisture) * temperature * (1.25 - local_biome) * (1.12 - ridge * 0.50),
            volcanic_core * (0.38 + ridge * 0.88) * (0.56 + local_biome),
        ];
        let (region_index, _) = weights
            .iter()
            .copied()
            .enumerate()
            .max_by(|(_, left), (_, right)| left.total_cmp(right))
            .unwrap_or((0, 0.0));
        let region = SurfaceRegion::ALL[region_index];
        let weight_sum = weights.iter().sum::<f32>().max(0.0001);
        let normalized = weights.map(|weight| weight / weight_sum);

        let base = 8.0 + continental * 27.0 + hills * 11.0 + detail * 4.0 + ridge * ridge * 10.0;
        let alpine = ridge.powi(3) * 34.0 + continental * 7.0;
        let terrace_height = (base / 6.0).floor() * 6.0 + detail * 2.0;
        let badlands = (terrace_height - base) * 0.82 + ridge * 8.0;
        let dune_wave = (x as f32 * 0.045 + self.value_2d(x, z, 180, 0xd447) * 7.0).sin();
        let dunes = dune_wave * 4.5 + (hills - 0.5) * 4.0;
        let volcanic = ridge.powi(4) * 25.0 + (volcanic_field - 0.5) * 10.0;
        let moor = (0.5 - hills) * 4.0 + ridge * 2.0;
        let forest = (moisture - 0.5) * 4.0 + detail * 2.0;
        // Continental noise becomes shelf, slope, then a genuinely navigable basin. The previous
        // single 18-voxel subtraction produced water only about 60 cm deep even across enormous
        // searches—visually an ocean, physically a puddle for a 1.78 m player.
        let ocean = smooth(((0.44 - continental) / 0.44).clamp(0.0, 1.0));
        let ocean_basin = ocean * 14.0 + ocean * ocean * 30.0;
        let height = (base
            + normalized[0] * forest
            + normalized[1] * moor
            + normalized[2] * alpine
            + normalized[3] * badlands
            + normalized[4] * dunes
            + normalized[5] * volcanic
            - ocean_basin)
            .round() as i32;
        let patch = self.value_2d(x, z, 42, 0x3f91);
        let aerosol = (normalized[3] * 0.62
            + normalized[4] * 0.30
            + normalized[5] * 0.92
            + (1.0 - moisture) * 0.12)
            .clamp(0.0, 1.0);
        let atmosphere = AtmosphereSample {
            humidity: moisture.clamp(0.0, 1.0),
            coldness: (1.0 - temperature).clamp(0.0, 1.0),
            aerosol,
            cloudiness: (moisture * 0.46
                + normalized[0] * 0.12
                + normalized[1] * 0.28
                + normalized[5] * 0.18)
                .clamp(0.0, 1.0),
            horizon_warmth: (temperature * 0.34
                + normalized[3] * 0.38
                + normalized[4] * 0.30
                + normalized[5] * 0.16)
                .clamp(0.0, 1.0),
            haze: (ocean * 0.24 + moisture * 0.20 + aerosol * 0.56).clamp(0.0, 1.0),
        };
        let material = if height < SEA_LEVEL_VOXELS + 3 {
            Material::Sand
        } else {
            match region {
                SurfaceRegion::VerdantForest => {
                    if moisture > 0.72 && patch > 0.68 {
                        Material::Moss
                    } else {
                        Material::Grass
                    }
                }
                SurfaceRegion::WindMoor => {
                    if ridge > 0.78 || patch < 0.10 {
                        Material::Limestone
                    } else {
                        Material::Grass
                    }
                }
                SurfaceRegion::Alpine => {
                    if temperature < 0.48 || height > 62 || patch > 0.74 {
                        Material::Snow
                    } else if ridge > 0.56 {
                        Material::Stone
                    } else {
                        Material::Limestone
                    }
                }
                SurfaceRegion::RedBadlands => {
                    if patch > 0.32 {
                        Material::RedSand
                    } else {
                        Material::Clay
                    }
                }
                SurfaceRegion::PaleDunes => {
                    if patch < 0.12 {
                        Material::Limestone
                    } else {
                        Material::Sand
                    }
                }
                SurfaceRegion::Volcanic => {
                    if patch > 0.18 {
                        Material::Basalt
                    } else {
                        Material::RedSand
                    }
                }
            }
        };

        ColumnProfile {
            height,
            moisture,
            temperature,
            ridge,
            region,
            material,
            atmosphere,
            route: None,
        }
    }

    fn apply_route_profile(self, x: i32, z: i32, mut profile: ColumnProfile) -> ColumnProfile {
        let Some(route) = sample_first_pilgrim_road(x, z) else {
            return profile;
        };
        let segment = usize::from(route.segment_index);
        let (Some(from), Some(to)) = (
            FIRST_PILGRIM_ROAD_NODES.get(segment).copied(),
            FIRST_PILGRIM_ROAD_NODES.get(segment + 1).copied(),
        ) else {
            return profile;
        };
        let target_height = from.y as f32 + (to.y - from.y) as f32 * route.segment_t;
        profile.height = (profile.height as f32
            + (target_height - profile.height as f32) * route.terrain_blend)
            .round() as i32;
        if route.core > 0.02 {
            let paving_along = (route.distance_along_voxels / 12.0).floor() as i32;
            let paving_lateral =
                ((route.signed_lateral_voxels + ROUTE_CORE_HALF_WIDTH_VOXELS) / 6.0).floor() as i32;
            profile.material = if self.hash(paving_along, 0, paving_lateral, 0x51ab_5eed) & 7 == 0 {
                Material::Stone
            } else {
                Material::Limestone
            };
        }
        profile.route = Some(route);
        profile
    }

    pub fn surface_height(self, x: i32, z: i32) -> i32 {
        self.column_profile(x, z).height
    }

    pub fn surface_sample(self, x: i32, z: i32) -> SurfaceSample {
        let profile = self.column_profile(x, z);
        SurfaceSample {
            height: profile.height,
            material: profile.material,
            water_level: (profile.height < SEA_LEVEL_VOXELS).then_some(SEA_LEVEL_VOXELS),
            region: profile.region,
            moisture: profile.moisture,
            temperature: profile.temperature,
            ridge: profile.ridge,
            atmosphere: profile.atmosphere,
            route: profile.route,
        }
    }

    /// Enumerates each deterministic skyline feature whose anchor lies in the supplied half-open
    /// X/Z bounds exactly once. Anchor ownership prevents cross-tile duplication at every LOD.
    pub fn skyline_features_anchored_in(self, bounds: [[i32; 2]; 2]) -> Vec<SkylineFeature> {
        let [[min_x, min_z], [max_x, max_z]] = bounds;
        if min_x >= max_x || min_z >= max_z {
            return Vec::new();
        }
        let min_cell_x = (min_x - 75).div_euclid(FEATURE_CELL_VOXELS);
        let max_cell_x = (max_x - 1 - 12).div_euclid(FEATURE_CELL_VOXELS);
        let min_cell_z = (min_z - 75).div_euclid(FEATURE_CELL_VOXELS);
        let max_cell_z = (max_z - 1 - 12).div_euclid(FEATURE_CELL_VOXELS);
        let mut features = Vec::new();
        for cell_z in min_cell_z..=max_cell_z {
            for cell_x in min_cell_x..=max_cell_x {
                let Some(feature) = self.skyline_feature(cell_x, cell_z) else {
                    continue;
                };
                if feature.anchor[0] >= min_x
                    && feature.anchor[0] < max_x
                    && feature.anchor[2] >= min_z
                    && feature.anchor[2] < max_z
                {
                    features.push(feature);
                }
            }
        }
        features
    }

    /// Returns procedural features whose canonical shape can own `coord`. The final material check
    /// remains cheap and lets edit invalidation target the anchor-owned derived mesh.
    pub fn skyline_features_at(self, coord: crate::VoxelCoord) -> Vec<SkylineFeature> {
        let cell_x = coord.x.div_euclid(FEATURE_CELL_VOXELS);
        let cell_z = coord.z.div_euclid(FEATURE_CELL_VOXELS);
        let mut features = Vec::new();
        for dz in -1..=1 {
            for dx in -1..=1 {
                let Some(feature) = self.skyline_feature(cell_x + dx, cell_z + dz) else {
                    continue;
                };
                if feature.material_at(coord).is_some() {
                    features.push(feature);
                }
            }
        }
        features
    }

    /// Finds a deterministic landmark for Rust-owned exploration/debug controls without exposing
    /// placement internals to the browser shell.
    pub fn nearest_skyline_feature(
        self,
        x: i32,
        z: i32,
        kind: SkylineFeatureKind,
        max_radius_cells: i32,
    ) -> Option<SkylineFeature> {
        self.nearest_skyline_feature_with_prominence(x, z, kind, 0, max_radius_cells)
    }

    /// Finds the nearest authored composition hero of one biome kind. This keeps the Rust debug tour
    /// focused on the larger silhouettes while ordinary callers can still request any landmark.
    pub fn nearest_prominent_skyline_feature(
        self,
        x: i32,
        z: i32,
        kind: SkylineFeatureKind,
        max_radius_cells: i32,
    ) -> Option<SkylineFeature> {
        self.nearest_skyline_feature_with_prominence(x, z, kind, 2, max_radius_cells)
    }

    fn nearest_skyline_feature_with_prominence(
        self,
        x: i32,
        z: i32,
        kind: SkylineFeatureKind,
        minimum_prominence: u8,
        max_radius_cells: i32,
    ) -> Option<SkylineFeature> {
        let center_x = x.div_euclid(FEATURE_CELL_VOXELS);
        let center_z = z.div_euclid(FEATURE_CELL_VOXELS);
        for radius in 0..=max_radius_cells.clamp(0, 256) {
            let mut closest = None;
            let mut consider = |cell_x: i32, cell_z: i32| {
                let Some(feature) = self.skyline_feature(cell_x, cell_z) else {
                    return;
                };
                if feature.kind != kind || feature.prominence < minimum_prominence {
                    return;
                }
                let dx = i64::from(feature.anchor[0]) - i64::from(x);
                let dz = i64::from(feature.anchor[2]) - i64::from(z);
                let distance = dx * dx + dz * dz;
                if closest.is_none_or(|(best, _)| distance < best) {
                    closest = Some((distance, feature));
                }
            };
            if radius == 0 {
                consider(center_x, center_z);
            } else {
                for dx in -radius..=radius {
                    consider(center_x + dx, center_z - radius);
                    consider(center_x + dx, center_z + radius);
                }
                for dz in (-radius + 1)..radius {
                    consider(center_x - radius, center_z + dz);
                    consider(center_x + radius, center_z + dz);
                }
            }
            if let Some((_, feature)) = closest {
                return Some(feature);
            }
        }
        None
    }

    fn decorate_features(self, chunk: &mut Chunk) {
        let origin = chunk.coord().world_origin();
        let max_x = origin[0] + CHUNK_EDGE as i32 - 1;
        let max_z = origin[2] + CHUNK_EDGE as i32 - 1;
        let min_cell_x =
            (origin[0] - FEATURE_MAX_RADIUS_VOXELS - 75).div_euclid(FEATURE_CELL_VOXELS);
        let max_cell_x = (max_x + FEATURE_MAX_RADIUS_VOXELS - 12).div_euclid(FEATURE_CELL_VOXELS);
        let min_cell_z =
            (origin[2] - FEATURE_MAX_RADIUS_VOXELS - 75).div_euclid(FEATURE_CELL_VOXELS);
        let max_cell_z = (max_z + FEATURE_MAX_RADIUS_VOXELS - 12).div_euclid(FEATURE_CELL_VOXELS);
        for cell_z in min_cell_z..=max_cell_z {
            for cell_x in min_cell_x..=max_cell_x {
                let Some(feature) = self.skyline_feature(cell_x, cell_z) else {
                    continue;
                };
                let bounds = feature.bounds();
                let min_y = bounds[0][1].max(origin[1]);
                let max_y = (bounds[1][1] - 1).min(origin[1] + CHUNK_EDGE as i32 - 1);
                for world_y in min_y..=max_y {
                    for world_z in bounds[0][2].max(origin[2])..=(bounds[1][2] - 1).min(max_z) {
                        for world_x in bounds[0][0].max(origin[0])..=(bounds[1][0] - 1).min(max_x) {
                            let local = [
                                (world_x - origin[0]) as usize,
                                (world_y - origin[1]) as usize,
                                (world_z - origin[2]) as usize,
                            ];
                            if chunk.get(local[0], local[1], local[2]).is_collidable() {
                                continue;
                            }
                            let material = feature
                                .material_at(crate::VoxelCoord::new(world_x, world_y, world_z));
                            if let Some(material) = material {
                                chunk.set(local[0], local[1], local[2], material);
                            }
                        }
                    }
                }
            }
        }
    }

    fn feature_anchor(self, cell_x: i32, cell_z: i32) -> (i32, i32, u64) {
        let hash = self.hash(cell_x, 0, cell_z, 0x7a11_5eed);
        let x_offset = 12 + (hash & 63) as i32;
        let z_offset = 12 + ((hash >> 8) & 63) as i32;
        (
            cell_x * FEATURE_CELL_VOXELS + x_offset,
            cell_z * FEATURE_CELL_VOXELS + z_offset,
            hash,
        )
    }

    fn feature_composition(self, cell_x: i32, cell_z: i32) -> FeatureComposition {
        let composition_x = cell_x.div_euclid(COMPOSITION_EDGE_FEATURE_CELLS);
        let composition_z = cell_z.div_euclid(COMPOSITION_EDGE_FEATURE_CELLS);
        let hash = self.hash(composition_x, 0, composition_z, 0xc06f_0517_10a5_1e5d);
        FeatureComposition::for_feature_cell(cell_x, cell_z, hash)
    }

    fn skyline_feature(self, cell_x: i32, cell_z: i32) -> Option<SkylineFeature> {
        if let Some(anchor) = first_pilgrim_route_anchor_for_feature_cell(cell_x, cell_z) {
            return self.route_landmark_feature(anchor);
        }
        let (anchor_x, anchor_z, hash) = self.feature_anchor(cell_x, cell_z);
        let surface = self.surface_sample(anchor_x, anchor_z);
        if surface.height < SEA_LEVEL_VOXELS || surface.water_level.is_some() {
            return None;
        }
        if surface.route.is_some_and(|route| {
            route.distance_to_route_voxels
                <= ROUTE_CORE_HALF_WIDTH_VOXELS + FEATURE_MAX_RADIUS_VOXELS as f32 + 3.0
        }) {
            return None;
        }
        let kind = match surface.region {
            SurfaceRegion::VerdantForest => SkylineFeatureKind::Broadleaf,
            SurfaceRegion::WindMoor => SkylineFeatureKind::MoorTor,
            SurfaceRegion::Alpine => SkylineFeatureKind::AlpineNeedle,
            SurfaceRegion::RedBadlands => SkylineFeatureKind::BadlandsHoodoo,
            SurfaceRegion::PaleDunes => SkylineFeatureKind::DuneArch,
            SurfaceRegion::Volcanic => SkylineFeatureKind::BasaltColumns,
        };
        let density_threshold: u8 = match kind {
            SkylineFeatureKind::Broadleaf => 224,
            SkylineFeatureKind::MoorTor => 92,
            SkylineFeatureKind::AlpineNeedle => 82,
            SkylineFeatureKind::BadlandsHoodoo => 88,
            SkylineFeatureKind::DuneArch => 64,
            SkylineFeatureKind::BasaltColumns => 112,
            SkylineFeatureKind::PilgrimCairn
            | SkylineFeatureKind::RouteWaystone
            | SkylineFeatureKind::RuinedArch => u8::MAX,
        };
        let composition = self.feature_composition(cell_x, cell_z);
        let influence = composition.influence(cell_x, cell_z);
        let effective_density = if influence.prominence == 2 {
            u8::MAX
        } else {
            (u16::from(density_threshold) * u16::from(influence.density) / u16::from(u8::MAX)) as u8
        };
        if effective_density == 0
            || ((hash >> 40) & 0xff) as u8 >= effective_density
            || (kind == SkylineFeatureKind::Broadleaf && surface.moisture < 0.30)
        {
            return None;
        }
        // `ridge` is already part of this authoritative column profile. Using it as the placement
        // constraint avoids four extra terrain-stack evaluations every time a hot column sampler
        // reconstructs its nine neighboring feature cells.
        let maximum_ridge = match kind {
            SkylineFeatureKind::Broadleaf | SkylineFeatureKind::DuneArch => 0.86,
            SkylineFeatureKind::MoorTor | SkylineFeatureKind::BadlandsHoodoo => 0.94,
            SkylineFeatureKind::AlpineNeedle | SkylineFeatureKind::BasaltColumns => 1.0,
            SkylineFeatureKind::PilgrimCairn
            | SkylineFeatureKind::RouteWaystone
            | SkylineFeatureKind::RuinedArch => 1.0,
        };
        if surface.ridge > maximum_ridge {
            return None;
        }
        let base_height = match kind {
            SkylineFeatureKind::Broadleaf => 25 + ((hash >> 16) & 15) as i32,
            SkylineFeatureKind::MoorTor => 18 + ((hash >> 16) & 11) as i32,
            SkylineFeatureKind::AlpineNeedle => 34 + ((hash >> 16) & 23) as i32,
            SkylineFeatureKind::BadlandsHoodoo => 24 + ((hash >> 16) & 15) as i32,
            SkylineFeatureKind::DuneArch => 18 + ((hash >> 16) & 7) as i32,
            SkylineFeatureKind::BasaltColumns => 24 + ((hash >> 16) & 19) as i32,
            SkylineFeatureKind::PilgrimCairn
            | SkylineFeatureKind::RouteWaystone
            | SkylineFeatureKind::RuinedArch => 0,
        };
        let height = base_height + base_height * i32::from(influence.prominence.min(2)) / 4;
        Some(SkylineFeature {
            id: SkylineFeatureId { cell_x, cell_z },
            kind,
            anchor: [anchor_x, surface.height, anchor_z],
            trunk_top: surface.height + height,
            orientation: ((hash >> 32) & 3) as u8,
            variant: ((hash >> 36) & 3) as u8,
            prominence: influence.prominence,
            route_landmark: None,
        })
    }

    fn route_landmark_feature(self, route_anchor: RouteAnchor) -> Option<SkylineFeature> {
        let [anchor_x, anchor_z] = route_anchor.anchor;
        let surface = self.surface_sample(anchor_x, anchor_z);
        if surface.water_level.is_some() || surface.height < SEA_LEVEL_VOXELS {
            return None;
        }
        let (kind, height, prominence) = match route_anchor.role {
            RouteAnchorRole::Cairn => (SkylineFeatureKind::PilgrimCairn, 11, 1),
            RouteAnchorRole::Waystone => (SkylineFeatureKind::RouteWaystone, 27, 1),
            RouteAnchorRole::RuinedArch => (SkylineFeatureKind::RuinedArch, 38, 2),
        };
        let route = sample_first_pilgrim_road(anchor_x, anchor_z)?;
        let orientation = if route.tangent[0].abs() >= route.tangent[1].abs() {
            1
        } else {
            0
        };
        Some(SkylineFeature {
            id: SkylineFeatureId {
                cell_x: route_anchor.feature_cell[0],
                cell_z: route_anchor.feature_cell[1],
            },
            kind,
            anchor: [anchor_x, surface.height, anchor_z],
            trunk_top: surface.height + height + i32::from(route_anchor.ordinal & 3),
            orientation,
            variant: (route_anchor.ordinal & 3) as u8,
            prominence,
            route_landmark: Some(RouteLandmarkId {
                route_id: route_anchor.route_id,
                ordinal: route_anchor.ordinal,
            }),
        })
    }

    fn fractal_2d(self, x: i32, z: i32, scale: i32, octaves: u32, salt: u64) -> f32 {
        let mut amplitude = 1.0;
        let mut total = 0.0;
        let mut weight = 0.0;
        let mut octave_scale = scale;
        for octave in 0..octaves {
            total += self.value_2d(x, z, octave_scale.max(2), salt + u64::from(octave)) * amplitude;
            weight += amplitude;
            amplitude *= 0.5;
            octave_scale /= 2;
        }
        total / weight
    }

    fn value_2d(self, x: i32, z: i32, scale: i32, salt: u64) -> f32 {
        let x0 = x.div_euclid(scale);
        let z0 = z.div_euclid(scale);
        let tx = smooth(x.rem_euclid(scale) as f32 / scale as f32);
        let tz = smooth(z.rem_euclid(scale) as f32 / scale as f32);
        let a = self.hash_unit(x0, 0, z0, salt);
        let b = self.hash_unit(x0 + 1, 0, z0, salt);
        let c = self.hash_unit(x0, 0, z0 + 1, salt);
        let d = self.hash_unit(x0 + 1, 0, z0 + 1, salt);
        lerp(lerp(a, b, tx), lerp(c, d, tx), tz)
    }

    fn value_3d(self, x: i32, y: i32, z: i32, scale: i32, salt: u64) -> f32 {
        let x0 = x.div_euclid(scale);
        let y0 = y.div_euclid(scale);
        let z0 = z.div_euclid(scale);
        let tx = smooth(x.rem_euclid(scale) as f32 / scale as f32);
        let ty = smooth(y.rem_euclid(scale) as f32 / scale as f32);
        let tz = smooth(z.rem_euclid(scale) as f32 / scale as f32);
        let plane = |dy: i32| {
            let a = self.hash_unit(x0, y0 + dy, z0, salt);
            let b = self.hash_unit(x0 + 1, y0 + dy, z0, salt);
            let c = self.hash_unit(x0, y0 + dy, z0 + 1, salt);
            let d = self.hash_unit(x0 + 1, y0 + dy, z0 + 1, salt);
            lerp(lerp(a, b, tx), lerp(c, d, tx), tz)
        };
        lerp(plane(0), plane(1), ty)
    }

    fn hash_unit(self, x: i32, y: i32, z: i32, salt: u64) -> f32 {
        (self.hash(x, y, z, salt) >> 40) as f32 / ((1u32 << 24) - 1) as f32
    }

    fn hash(self, x: i32, y: i32, z: i32, salt: u64) -> u64 {
        let mut value = self.seed ^ salt;
        value ^= (x as i64 as u64).wrapping_mul(0x9e37_79b1_85eb_ca87);
        value ^= (y as i64 as u64).wrapping_mul(0xc2b2_ae3d_27d4_eb4f);
        value ^= (z as i64 as u64).wrapping_mul(0x1656_67b1_9e37_79f9);
        value ^= value >> 30;
        value = value.wrapping_mul(0xbf58_476d_1ce4_e5b9);
        value ^= value >> 27;
        value = value.wrapping_mul(0x94d0_49bb_1331_11eb);
        value ^= value >> 31;
        value
    }
}

fn smooth(value: f32) -> f32 {
    value * value * (3.0 - 2.0 * value)
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::VoxelCoord;
    use std::collections::BTreeSet;

    fn flooded_column(generator: Generator) -> (i32, i32, SurfaceSample) {
        let mut minimum = i32::MAX;
        for z in (-12_000..=12_000).step_by(37) {
            for x in (-12_000..=12_000).step_by(37) {
                let sample = generator.surface_sample(x, z);
                minimum = minimum.min(sample.height);
                if sample.water_level.is_some() {
                    return (x, z, sample);
                }
            }
        }
        panic!("fixed seed should generate a flooded coastal column; minimum was {minimum}");
    }

    #[test]
    fn generation_is_deterministic_across_chunk_boundaries() {
        let generator = Generator::new(0x5eed);
        let left = generator.generate_chunk(ChunkCoord::new(-1, 0, 0));
        let right = generator.generate_chunk(ChunkCoord::new(0, 0, 0));
        assert_eq!(left.get(31, 17, 9), generator.sample(-1, 17, 9));
        assert_eq!(right.get(0, 17, 9), generator.sample(0, 17, 9));
        assert_ne!(left.voxels(), right.voxels());
    }

    #[test]
    fn cached_column_generation_matches_random_access_sampling() {
        let generator = Generator::new(0x5eed);
        let coord = ChunkCoord::new(-2, 0, 3);
        let chunk = generator.generate_chunk(coord);
        let origin = coord.world_origin();
        for y in [0, 5, 15, 31] {
            for z in [0, 11, 23, 31] {
                for x in [0, 7, 15, 31] {
                    assert_eq!(
                        chunk.get(x, y, z),
                        generator.sample(
                            origin[0] + x as i32,
                            origin[1] + y as i32,
                            origin[2] + z as i32,
                        )
                    );
                }
            }
        }
    }

    #[test]
    fn generated_region_matches_random_access_across_feature_cells_and_negative_space() {
        let generator = Generator::new(0x5eed_cafe);
        for [min_x, min_z] in [[-113, -101], [79, 91], [18_015, 12_895]] {
            let region = generator.region(min_x, min_z, 34, 34);
            for z in min_z..min_z + 34 {
                for x in min_x..min_x + 34 {
                    for y in [-17, 0, 9, 10, 24, 48] {
                        assert_eq!(region.sample(x, y, z), generator.sample(x, y, z));
                    }
                }
            }
        }
    }

    #[test]
    fn different_seeds_produce_different_chunks() {
        let coord = ChunkCoord::new(1, 0, -2);
        assert_ne!(
            Generator::new(1).generate_chunk(coord),
            Generator::new(2).generate_chunk(coord)
        );
    }

    #[test]
    fn macro_terrain_varies_over_real_world_distances() {
        let generator = Generator::new(0x5eed);
        let nearby_delta = (generator.surface_height(0, 0) - generator.surface_height(10, 0)).abs();
        let distant_delta =
            (generator.surface_height(0, 0) - generator.surface_height(2_000, 0)).abs();
        assert!(
            nearby_delta <= 8,
            "one metre should not cross a macro biome"
        );
        assert!(distant_delta >= nearby_delta);
    }

    #[test]
    fn regional_surface_catalog_is_represented_and_self_consistent() {
        let generator = Generator::new(0x5eed);
        let mut regions = BTreeSet::new();
        for z in (-12_000..=12_000).step_by(389) {
            for x in (-12_000..=12_000).step_by(389) {
                let sample = generator.surface_sample(x, z);
                regions.insert(sample.region);
                assert!(sample.material.is_collidable());
                let profile = generator.column_profile(x, z);
                assert_eq!(
                    generator.sample_terrain_with_profile(x, sample.height, z, profile),
                    sample.material
                );
                assert_eq!(
                    generator.sample_terrain_with_profile(x, sample.height + 1, z, profile),
                    if sample.height < SEA_LEVEL_VOXELS {
                        Material::Water
                    } else {
                        Material::Air
                    }
                );
            }
        }
        assert_eq!(regions, SurfaceRegion::ALL.into_iter().collect());
    }

    #[test]
    fn continuous_atmosphere_is_normalized_smooth_and_regionally_distinct() {
        let generator = Generator::new(0x5eed_cafe);
        let mut representative: [Option<AtmosphereSample>; 6] = [None; 6];
        let mut maximum_adjacent_delta = 0.0f32;
        for z in (-12_000..=12_000).step_by(389) {
            for x in (-12_000..=12_000).step_by(389) {
                let sample = generator.surface_sample(x, z);
                assert!(sample.atmosphere.is_finite_and_normalized());
                representative[sample.region as usize].get_or_insert(sample.atmosphere);
                let adjacent = generator.surface_sample(x + 1, z).atmosphere;
                maximum_adjacent_delta = sample
                    .atmosphere
                    .components()
                    .into_iter()
                    .zip(adjacent.components())
                    .fold(maximum_adjacent_delta, |maximum, (left, right)| {
                        maximum.max((left - right).abs())
                    });
            }
        }
        assert!(
            maximum_adjacent_delta < 0.025,
            "one-voxel atmosphere jump reached {maximum_adjacent_delta}"
        );
        assert!(representative.iter().all(Option::is_some));
        let representative = representative.map(Option::unwrap);
        for left in 0..representative.len() {
            for right in left + 1..representative.len() {
                let distance: f32 = representative[left]
                    .components()
                    .into_iter()
                    .zip(representative[right].components())
                    .map(|(left, right)| (left - right).abs())
                    .sum();
                assert!(
                    distance > 0.08,
                    "region atmosphere fingerprints {left} and {right} nearly matched"
                );
            }
        }
        let midpoint = representative[0].lerp(representative[5], 0.5);
        assert!(midpoint.is_finite_and_normalized());
    }

    #[test]
    fn regional_surface_checksum_tracks_generator_version() {
        let generator = Generator::new(0x5eed_cafe);
        let mut checksum = u64::from(GENERATOR_VERSION);
        for z in (-768..=768).step_by(37) {
            for x in (-768..=768).step_by(37) {
                let sample = generator.surface_sample(x, z);
                for value in [
                    sample.height as i64 as u64,
                    u64::from(sample.material.id()),
                    sample.region as u64,
                    sample.water_level.unwrap_or(i32::MIN) as i64 as u64,
                ] {
                    checksum ^= value;
                    checksum = checksum.wrapping_mul(0x100_0000_01b3);
                }
            }
        }
        assert_eq!(checksum, 0x3d1c_bc5e_e75f_c0a6);
    }

    #[test]
    fn ocean_fills_only_above_low_terrain_through_the_versioned_sea_level() {
        let generator = Generator::new(0x5eed_cafe);
        let (x, z, sample) = flooded_column(generator);
        assert!(sample.height < SEA_LEVEL_VOXELS);
        assert_eq!(sample.water_level, Some(SEA_LEVEL_VOXELS));
        assert_eq!(generator.sample(x, sample.height, z), sample.material);
        assert_ne!(generator.sample(x, sample.height - 1, z), Material::Water);
        assert_eq!(generator.sample(x, sample.height + 1, z), Material::Water);
        assert_eq!(generator.sample(x, SEA_LEVEL_VOXELS, z), Material::Water);
        assert_eq!(generator.sample(x, SEA_LEVEL_VOXELS + 1, z), Material::Air);

        let chunk_coord = VoxelCoord::new(x, SEA_LEVEL_VOXELS, z).chunk();
        let chunk = generator.generate_chunk(chunk_coord);
        let local = VoxelCoord::new(x, SEA_LEVEL_VOXELS, z).local();
        assert_eq!(chunk.get(local[0], local[1], local[2]), Material::Water);
    }

    #[test]
    fn ocean_bathymetry_contains_player_deep_navigable_water() {
        let sample = Generator::new(0x5eed_cafe).surface_sample(18_016, 12_896);
        let water_level = sample
            .water_level
            .expect("showcase basin should be flooded");
        assert!(
            water_level - sample.height >= 20,
            "a full-height player needs at least two metres of canonical water"
        );
    }

    #[test]
    fn procedural_landmarks_stay_dry_and_inside_their_cells() {
        let generator = Generator::new(0x5eed_cafe);
        for feature in generator.skyline_features_anchored_in([[-4_096, -4_096], [4_096, 4_096]]) {
            assert!(feature.anchor[1] >= SEA_LEVEL_VOXELS);
            let cell_min_x = feature.id.cell_x * FEATURE_CELL_VOXELS;
            let cell_min_z = feature.id.cell_z * FEATURE_CELL_VOXELS;
            let [min, max] = feature.bounds();
            assert!(min[0] >= cell_min_x && max[0] <= cell_min_x + FEATURE_CELL_VOXELS);
            assert!(min[2] >= cell_min_z && max[2] <= cell_min_z + FEATURE_CELL_VOXELS);
        }
    }

    #[test]
    fn broadleaf_landmarks_are_deterministic_editable_voxels() {
        let generator = Generator::new(0x5eed);
        let feature = generator
            .skyline_features_anchored_in([[-1_024, -1_024], [1_024, 1_024]])
            .into_iter()
            .find(|feature| feature.kind == SkylineFeatureKind::Broadleaf)
            .expect("fixed seed should produce a nearby broadleaf landmark");
        let [x, ground, z] = feature.anchor;
        assert_eq!(generator.sample(x, ground + 1, z), Material::Wood);
        assert_eq!(generator.sample(x, feature.trunk_top, z), Material::Wood);
        assert_eq!(
            generator.sample(x + 5, feature.trunk_top - 3, z),
            Material::Leaves
        );
    }

    #[test]
    fn pilgrim_route_landmarks_override_ambient_placement_with_stable_identity() {
        let generator = Generator::new(0x5eed_cafe);
        let count = crate::first_pilgrim_route_anchor_count();
        assert_eq!(count, 5);
        let mut kinds = BTreeSet::new();

        for ordinal in 0..count {
            let anchor = crate::first_pilgrim_route_anchor(ordinal).unwrap();
            let feature = generator
                .skyline_feature(anchor.feature_cell[0], anchor.feature_cell[1])
                .expect("every authored route token should own its feature cell");
            let expected_kind = match anchor.role {
                RouteAnchorRole::Cairn => SkylineFeatureKind::PilgrimCairn,
                RouteAnchorRole::Waystone => SkylineFeatureKind::RouteWaystone,
                RouteAnchorRole::RuinedArch => SkylineFeatureKind::RuinedArch,
            };
            assert_eq!(feature.kind, expected_kind);
            assert_eq!(feature.anchor[0], anchor.anchor[0]);
            assert_eq!(feature.anchor[2], anchor.anchor[1]);
            assert_eq!(
                feature.route_landmark,
                Some(RouteLandmarkId {
                    route_id: anchor.route_id,
                    ordinal,
                })
            );
            assert!(feature.prominence >= 1);
            kinds.insert(feature.kind);

            let [min, max] = feature.bounds();
            let cell_min = [
                anchor.feature_cell[0] * FEATURE_CELL_VOXELS,
                anchor.feature_cell[1] * FEATURE_CELL_VOXELS,
            ];
            assert!(min[0] >= cell_min[0] && max[0] <= cell_min[0] + FEATURE_CELL_VOXELS);
            assert!(min[2] >= cell_min[1] && max[2] <= cell_min[1] + FEATURE_CELL_VOXELS);

            let [min, max] = feature.bounds();
            let (voxel, material) = (min[1]..max[1])
                .flat_map(|y| {
                    (min[2]..max[2])
                        .flat_map(move |z| (min[0]..max[0]).map(move |x| VoxelCoord::new(x, y, z)))
                })
                .find_map(|voxel| {
                    feature.material_at(voxel).and_then(|material| {
                        (generator.sample(voxel.x, voxel.y, voxel.z) == material)
                            .then_some((voxel, material))
                    })
                })
                .expect("the route landmark should contain visible canonical structure");
            assert_eq!(generator.sample(voxel.x, voxel.y, voxel.z), material);
            let chunk = generator.generate_chunk(voxel.chunk());
            let local = voxel.local();
            assert_eq!(chunk.get(local[0], local[1], local[2]), material);
            let region = generator.region(voxel.x - 1, voxel.z - 1, 3, 3);
            assert_eq!(region.sample(voxel.x, voxel.y, voxel.z), material);
        }

        assert_eq!(
            kinds,
            BTreeSet::from([
                SkylineFeatureKind::PilgrimCairn,
                SkylineFeatureKind::RouteWaystone,
                SkylineFeatureKind::RuinedArch,
            ])
        );
    }

    #[test]
    fn road_destination_retains_its_badlands_hoodoo_silhouette() {
        let generator = Generator::new(0x5eed_cafe);
        let destination = crate::FIRST_PILGRIM_ROAD_NODES.last().unwrap();
        let nearby = generator.skyline_features_anchored_in([
            [destination.x - 512, destination.z - 512],
            [destination.x + 513, destination.z + 513],
        ]);
        let hoodoo = nearby
            .iter()
            .copied()
            .filter(|feature| {
                feature.kind == SkylineFeatureKind::BadlandsHoodoo
                    && feature.route_landmark.is_none()
            })
            .min_by_key(|feature| {
                let dx = feature.anchor[0] - destination.x;
                let dz = feature.anchor[2] - destination.z;
                dx * dx + dz * dz
            })
            .unwrap_or_else(|| panic!("destination catalog had no hoodoo: {nearby:?}"));
        let dx = hoodoo.anchor[0] - destination.x;
        let dz = hoodoo.anchor[2] - destination.z;
        assert!(dx * dx + dz * dz <= 100 * 100);
    }

    #[test]
    fn analytic_skyline_descriptor_is_the_canonical_broadleaf_source() {
        let generator = Generator::new(0x5eed);
        let feature = generator
            .skyline_features_anchored_in([[-1_024, -1_024], [1_024, 1_024]])
            .into_iter()
            .find(|feature| feature.kind == SkylineFeatureKind::Broadleaf)
            .expect("fixed seed should place a nearby tree");
        let probes = [
            VoxelCoord::new(feature.anchor[0], feature.anchor[1] + 1, feature.anchor[2]),
            VoxelCoord::new(feature.anchor[0], feature.trunk_top, feature.anchor[2]),
            VoxelCoord::new(
                feature.anchor[0] + 5,
                feature.trunk_top - 3,
                feature.anchor[2],
            ),
        ];
        assert_eq!(feature.material_at(probes[0]), Some(Material::Wood));
        assert_eq!(feature.material_at(probes[1]), Some(Material::Wood));
        assert_eq!(feature.material_at(probes[2]), Some(Material::Leaves));
        for probe in probes {
            assert_eq!(
                generator.sample(probe.x, probe.y, probe.z),
                feature.material_at(probe).unwrap()
            );
        }

        let one_voxel_anchor_bounds = [
            [feature.anchor[0], feature.anchor[2]],
            [feature.anchor[0] + 1, feature.anchor[2] + 1],
        ];
        assert_eq!(
            generator.skyline_features_anchored_in(one_voxel_anchor_bounds),
            [feature]
        );
        let bounds = feature.bounds();
        assert_eq!(
            feature.material_at(VoxelCoord::new(
                bounds[1][0],
                feature.trunk_top,
                feature.anchor[2]
            )),
            None
        );
    }

    #[test]
    fn every_surface_region_has_a_distinct_landmark_archetype() {
        let generator = Generator::new(0x5eed_cafe);
        let mut kinds = BTreeSet::new();
        'catalog: for z in (-12_000i32..=12_000).step_by(389) {
            for x in (-12_000i32..=12_000).step_by(389) {
                let cell_x = x.div_euclid(FEATURE_CELL_VOXELS);
                let cell_z = z.div_euclid(FEATURE_CELL_VOXELS);
                for dz in -4..=4 {
                    for dx in -4..=4 {
                        if let Some(feature) = generator.skyline_feature(cell_x + dx, cell_z + dz) {
                            kinds.insert(feature.kind);
                        }
                    }
                }
                if kinds.len() == SkylineFeatureKind::ALL.len() {
                    break 'catalog;
                }
            }
        }
        assert_eq!(kinds, SkylineFeatureKind::ALL.into_iter().collect());
    }

    #[test]
    fn composed_landmark_catalog_is_stable_and_bounded() {
        let generator = Generator::new(0x5eed_cafe);
        let mut modes = BTreeSet::new();
        let mut prominence_counts = [0usize; 3];
        let mut checksum = u64::from(GENERATOR_VERSION);
        for cell_z in -32..=32 {
            for cell_x in -32..=32 {
                modes.insert(generator.feature_composition(cell_x, cell_z).mode);
                let Some(feature) = generator.skyline_feature(cell_x, cell_z) else {
                    continue;
                };
                assert!(feature.prominence <= 2);
                prominence_counts[usize::from(feature.prominence)] += 1;
                let [min, max] = feature.bounds();
                let cell_min_x = cell_x * FEATURE_CELL_VOXELS;
                let cell_min_z = cell_z * FEATURE_CELL_VOXELS;
                assert!(min[0] >= cell_min_x && max[0] <= cell_min_x + FEATURE_CELL_VOXELS);
                assert!(min[2] >= cell_min_z && max[2] <= cell_min_z + FEATURE_CELL_VOXELS);
                for value in [
                    cell_x as i64 as u64,
                    cell_z as i64 as u64,
                    feature.kind as u64,
                    feature.anchor[0] as i64 as u64,
                    feature.anchor[1] as i64 as u64,
                    feature.anchor[2] as i64 as u64,
                    feature.trunk_top as i64 as u64,
                    u64::from(feature.orientation),
                    u64::from(feature.variant),
                    u64::from(feature.prominence),
                ] {
                    checksum ^= value;
                    checksum = checksum.wrapping_mul(0x100_0000_01b3);
                }
            }
        }
        assert_eq!(
            modes,
            crate::FeatureCompositionMode::ALL.into_iter().collect()
        );
        assert!(prominence_counts.into_iter().all(|count| count > 0));
        assert_eq!(checksum, 0x251b_911c_980d_e284);
    }

    #[test]
    fn prominent_landmark_search_returns_composition_heroes() {
        let generator = Generator::new(0x5eed_cafe);
        for kind in SkylineFeatureKind::REGIONAL {
            let feature = generator
                .nearest_prominent_skyline_feature(0, 0, kind, 192)
                .expect("the fixed catalog should contain a hero of every regional kind");
            assert_eq!(feature.kind, kind);
            assert_eq!(feature.prominence, 2);
        }
    }

    #[test]
    fn pilgrim_road_is_dry_continuous_editable_ten_centimetre_terrain() {
        let generator = Generator::new(0x5eed_cafe);
        let length = crate::first_pilgrim_road_length_voxels() as i32;
        let mut previous: Option<i32> = None;
        let mut previous_point = None;
        let mut max_step = 0;
        let mut max_cut = 0;
        let mut max_fill = 0;
        let mut sampled_columns = 0;
        for distance in 0..=length {
            let (point, tangent) = crate::first_pilgrim_road_point_at_distance(distance as f32)
                .expect("road distance must resolve to its authored polyline");
            let point = [point[0].round() as i32, point[1].round() as i32];
            if previous_point == Some(point) {
                continue;
            }
            previous_point = Some(point);
            let sample = generator.surface_sample(point[0], point[1]);
            let route = sample.route.expect("centerline must retain route identity");
            assert!(route.core > 0.5);
            assert!(sample.water_level.is_none());
            assert!(matches!(
                sample.material,
                Material::Limestone | Material::Stone
            ));
            assert_eq!(
                generator.sample(point[0], sample.height, point[1]),
                sample.material
            );
            assert_eq!(
                generator.sample(point[0], sample.height + 1, point[1]),
                Material::Air
            );
            let natural = generator.natural_column_profile(point[0], point[1]);
            max_cut = max_cut.max(natural.height - sample.height);
            max_fill = max_fill.max(sample.height - natural.height);
            if let Some(previous) = previous {
                max_step = max_step.max((sample.height - previous).abs());
            }
            previous = Some(sample.height);
            sampled_columns += 1;

            // The player's 0.56 m diameter fits entirely on the graded bed without a cross-slope snag.
            for lateral in [-3.0, 3.0] {
                let x = (point[0] as f32 - tangent[1] * lateral).round() as i32;
                let z = (point[1] as f32 + tangent[0] * lateral).round() as i32;
                let shoulder = generator.surface_sample(x, z);
                assert!((shoulder.height - sample.height).abs() <= 1);
                assert!(shoulder.water_level.is_none());
            }
        }
        assert!(sampled_columns > 1_000);
        assert!(
            max_step <= 3,
            "road longitudinal step was {max_step} voxels"
        );
        assert!(max_cut <= 3, "road cut reached {max_cut} voxels");
        assert!(max_fill <= 2, "road fill reached {max_fill} voxels");
    }

    #[test]
    fn route_surface_matches_chunk_region_and_sparse_edit_authority() {
        let generator = Generator::new(0x5eed_cafe);
        let length = crate::first_pilgrim_road_length_voxels();
        let probes = [
            crate::first_pilgrim_road_point_at_distance(0.0).unwrap().0,
            crate::first_pilgrim_road_point_at_distance(length * 0.5)
                .unwrap()
                .0,
            crate::first_pilgrim_road_point_at_distance(length - 1.0)
                .unwrap()
                .0,
        ];
        for point in probes {
            let x = point[0].round() as i32;
            let z = point[1].round() as i32;
            let surface = generator.surface_sample(x, z);
            let coord = crate::VoxelCoord::new(x, surface.height, z);
            let chunk = generator.generate_chunk(coord.chunk());
            let local = coord.local();
            assert_eq!(chunk.get(local[0], local[1], local[2]), surface.material);
            let region = generator.region(x - 1, z - 1, 3, 3);
            assert_eq!(region.sample(x, surface.height, z), surface.material);

            let mut edits = crate::EditMap::default();
            edits.set(generator, coord, Material::Air);
            assert_eq!(edits.sample(generator, coord), Material::Air);
            edits.set(generator, coord, surface.material);
            assert!(edits.is_empty());
        }
    }

    #[test]
    fn terrain_outside_route_shoulder_is_identical_to_natural_generation() {
        let generator = Generator::new(0x5eed_cafe);
        for distance in (0..crate::first_pilgrim_road_length_voxels() as i32).step_by(73) {
            let (point, tangent) =
                crate::first_pilgrim_road_point_at_distance(distance as f32).unwrap();
            for side in [-1.0, 1.0] {
                let offset = crate::ROUTE_SHOULDER_WIDTH_VOXELS + 2.0;
                let x = (point[0] - tangent[1] * offset * side).round() as i32;
                let z = (point[1] + tangent[0] * offset * side).round() as i32;
                if sample_first_pilgrim_road(x, z).is_some() {
                    // Near a bend, the other segment can legitimately own this sample.
                    continue;
                }
                let actual = generator.column_profile(x, z);
                let natural = generator.natural_column_profile(x, z);
                assert_eq!(actual.height, natural.height);
                assert_eq!(actual.material, natural.material);
                assert_eq!(actual.region, natural.region);
                assert_eq!(actual.moisture, natural.moisture);
                assert_eq!(actual.temperature, natural.temperature);
                assert_eq!(actual.ridge, natural.ridge);
                assert!(actual.route.is_none());
            }
        }
    }
}
