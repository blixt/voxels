use crate::{CHUNK_EDGE, Chunk, ChunkCoord, Material};

/// Generator version is part of world identity. Changing terrain semantics requires incrementing it.
pub const GENERATOR_VERSION: u32 = 6;
pub const SEA_LEVEL_VOXELS: i32 = 10;

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
}

const TREE_CELL_VOXELS: i32 = 96;
const TREE_CROWN_RADIUS_VOXELS: i32 = 8;

/// Stable procedural identity for an analytic feature that remains readable in surface LODs.
/// The placement cell is sufficient to reconstruct the feature from the generator seed.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SkylineFeatureId {
    pub cell_x: i32,
    pub cell_z: i32,
}

/// One deterministic procedural feature shared by canonical voxel generation and disposable
/// skyline proxies. Bounds are half-open canonical 10 cm voxel coordinates.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SkylineFeature {
    pub id: SkylineFeatureId,
    pub anchor: [i32; 3],
    pub trunk_top: i32,
}

impl SkylineFeature {
    pub const fn bounds(self) -> [[i32; 3]; 2] {
        [
            [
                self.anchor[0] - TREE_CROWN_RADIUS_VOXELS,
                self.anchor[1] + 1,
                self.anchor[2] - TREE_CROWN_RADIUS_VOXELS,
            ],
            [
                self.anchor[0] + TREE_CROWN_RADIUS_VOXELS + 1,
                self.trunk_top + 3,
                self.anchor[2] + TREE_CROWN_RADIUS_VOXELS + 1,
            ],
        ]
    }

    pub fn material_at(self, coord: crate::VoxelCoord) -> Option<Material> {
        let dx = coord.x - self.anchor[0];
        let dz = coord.z - self.anchor[2];
        if coord.y > self.anchor[1] && coord.y <= self.trunk_top && dx.abs() <= 1 && dz.abs() <= 1 {
            return Some(Material::Wood);
        }
        let dy = coord.y - (self.trunk_top - 3);
        let horizontal_radius = 9 - dy.abs() / 2;
        let distance_squared = dx * dx + dz * dz + (dy * dy) / 2;
        ((-7..=5).contains(&dy)
            && dx.abs() <= horizontal_radius
            && dz.abs() <= horizontal_radius
            && distance_squared <= 78)
            .then_some(Material::Leaves)
    }

    pub const fn contains_xz(self, x: i32, z: i32) -> bool {
        let bounds = self.bounds();
        x >= bounds[0][0] && x < bounds[1][0] && z >= bounds[0][2] && z < bounds[1][2]
    }
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
    trees: [SkylineFeature; 9],
    tree_count: u8,
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
        self.trees[..usize::from(self.tree_count)]
            .iter()
            .find_map(|tree| tree.material_at(crate::VoxelCoord::new(self.x, y, self.z)))
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
        self.decorate_trees(&mut chunk);
        chunk
    }

    pub fn sample(self, x: i32, y: i32, z: i32) -> Material {
        self.column(x, z).sample(y)
    }

    pub fn column(self, x: i32, z: i32) -> GeneratedColumn {
        let mut trees = [SkylineFeature::default(); 9];
        let mut tree_count = 0u8;
        let cell_x = x.div_euclid(TREE_CELL_VOXELS);
        let cell_z = z.div_euclid(TREE_CELL_VOXELS);
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
                trees[usize::from(tree_count)] = feature;
                tree_count += 1;
            }
        }
        GeneratedColumn {
            generator: self,
            x,
            z,
            profile: self.column_profile(x, z),
            trees,
            tree_count,
        }
    }

    pub fn region(self, min_x: i32, min_z: i32, width: usize, depth: usize) -> GeneratedRegion {
        let max_x =
            min_x.saturating_add(i32::try_from(width.saturating_sub(1)).unwrap_or(i32::MAX));
        let max_z =
            min_z.saturating_add(i32::try_from(depth.saturating_sub(1)).unwrap_or(i32::MAX));
        let min_cell_x = min_x.div_euclid(TREE_CELL_VOXELS) - 1;
        let max_cell_x = max_x.div_euclid(TREE_CELL_VOXELS) + 1;
        let min_cell_z = min_z.div_euclid(TREE_CELL_VOXELS) - 1;
        let max_cell_z = max_z.div_euclid(TREE_CELL_VOXELS) + 1;
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
                let mut trees = [SkylineFeature::default(); 9];
                let mut tree_count = 0u8;
                for feature in features
                    .iter()
                    .copied()
                    .filter(|feature| feature.contains_xz(x, z))
                {
                    trees[usize::from(tree_count)] = feature;
                    tree_count += 1;
                }
                columns.push(GeneratedColumn {
                    generator: self,
                    x,
                    z,
                    profile: self.column_profile(x, z),
                    trees,
                    tree_count,
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
        }
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
        }
    }

    /// Enumerates each deterministic skyline feature whose anchor lies in the supplied half-open
    /// X/Z bounds exactly once. Anchor ownership prevents cross-tile duplication at every LOD.
    pub fn skyline_features_anchored_in(self, bounds: [[i32; 2]; 2]) -> Vec<SkylineFeature> {
        let [[min_x, min_z], [max_x, max_z]] = bounds;
        if min_x >= max_x || min_z >= max_z {
            return Vec::new();
        }
        let min_cell_x = (min_x - 75).div_euclid(TREE_CELL_VOXELS);
        let max_cell_x = (max_x - 1 - 12).div_euclid(TREE_CELL_VOXELS);
        let min_cell_z = (min_z - 75).div_euclid(TREE_CELL_VOXELS);
        let max_cell_z = (max_z - 1 - 12).div_euclid(TREE_CELL_VOXELS);
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
        let cell_x = coord.x.div_euclid(TREE_CELL_VOXELS);
        let cell_z = coord.z.div_euclid(TREE_CELL_VOXELS);
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

    fn decorate_trees(self, chunk: &mut Chunk) {
        let origin = chunk.coord().world_origin();
        let max_x = origin[0] + CHUNK_EDGE as i32 - 1;
        let max_z = origin[2] + CHUNK_EDGE as i32 - 1;
        let min_cell_x = (origin[0] - TREE_CROWN_RADIUS_VOXELS - 75).div_euclid(TREE_CELL_VOXELS);
        let max_cell_x = (max_x + TREE_CROWN_RADIUS_VOXELS - 12).div_euclid(TREE_CELL_VOXELS);
        let min_cell_z = (origin[2] - TREE_CROWN_RADIUS_VOXELS - 75).div_euclid(TREE_CELL_VOXELS);
        let max_cell_z = (max_z + TREE_CROWN_RADIUS_VOXELS - 12).div_euclid(TREE_CELL_VOXELS);
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

    fn tree_grows_on(self, x: i32, z: i32, surface: SurfaceSample) -> bool {
        match surface.region {
            SurfaceRegion::VerdantForest => surface.moisture >= 0.30,
            SurfaceRegion::WindMoor => {
                surface.moisture >= 0.42 && self.value_2d(x, z, 310, 0x781d) > 0.66
            }
            _ => false,
        }
    }

    fn tree_anchor(self, cell_x: i32, cell_z: i32) -> (i32, i32, i32) {
        let hash = self.hash(cell_x, 0, cell_z, 0x7a11_5eed);
        let x_offset = 12 + (hash & 63) as i32;
        let z_offset = 12 + ((hash >> 8) & 63) as i32;
        let height = 25 + ((hash >> 16) & 15) as i32;
        (
            cell_x * TREE_CELL_VOXELS + x_offset,
            cell_z * TREE_CELL_VOXELS + z_offset,
            height,
        )
    }

    fn skyline_feature(self, cell_x: i32, cell_z: i32) -> Option<SkylineFeature> {
        let (anchor_x, anchor_z, height) = self.tree_anchor(cell_x, cell_z);
        let surface = self.surface_sample(anchor_x, anchor_z);
        (surface.height >= SEA_LEVEL_VOXELS && self.tree_grows_on(anchor_x, anchor_z, surface))
            .then_some(SkylineFeature {
                id: SkylineFeatureId { cell_x, cell_z },
                anchor: [anchor_x, surface.height, anchor_z],
                trunk_top: surface.height + height,
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
    fn generated_region_matches_random_access_across_tree_cells_and_negative_space() {
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
        assert_eq!(checksum, 0x39a3_a092_5fe2_08a1);
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
    fn procedural_tree_anchors_stay_on_dry_land() {
        let generator = Generator::new(0x5eed_cafe);
        for feature in generator.skyline_features_anchored_in([[-4_096, -4_096], [4_096, 4_096]]) {
            assert!(feature.anchor[1] >= SEA_LEVEL_VOXELS);
        }
    }

    #[test]
    fn trees_are_deterministic_editable_voxels() {
        let generator = Generator::new(0x5eed);
        let mut found = None;
        for cell_z in -4..=4 {
            for cell_x in -4..=4 {
                let (x, z, height) = generator.tree_anchor(cell_x, cell_z);
                let ground = generator.surface_height(x, z);
                if generator.sample(x, ground + 1, z) == Material::Wood {
                    found = Some((x, ground, z, height));
                    break;
                }
            }
        }
        assert!(
            found.is_some(),
            "seed should produce at least one nearby tree"
        );
        let Some((x, ground, z, height)) = found else {
            return;
        };
        assert_eq!(generator.sample(x, ground + 1, z), Material::Wood);
        assert_eq!(generator.sample(x, ground + height, z), Material::Wood);
        assert_eq!(
            generator.sample(x + 5, ground + height - 3, z),
            Material::Leaves
        );
    }

    #[test]
    fn analytic_skyline_descriptor_is_the_canonical_tree_source() {
        let generator = Generator::new(0x5eed);
        let feature = generator
            .skyline_features_anchored_in([[-512, -512], [512, 512]])
            .into_iter()
            .next()
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
}
