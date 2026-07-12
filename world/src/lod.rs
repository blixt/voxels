use crate::mesh::{FACE_NEG_X, FACE_NEG_Y, FACE_NEG_Z, FACE_POS_X, FACE_POS_Y, FACE_POS_Z};
use crate::{
    EditMap, FEATURE_MAX_RADIUS_VOXELS, Generator, Material, SEA_LEVEL_VOXELS, SkylineFeature,
    SkylineFeatureKind, VoxelCoord,
};
use std::ops::Range;

/// Every surface LOD tile contains the same number of cells. Increasing the level therefore
/// increases world coverage without increasing generation or upload work per tile.
pub const SURFACE_TILE_EDGE_CELLS: i32 = 32;

/// Surface tiles are emitted as independently addressable patches. Keeping patches much smaller
/// than a streamed tile lets the renderer select geometric clipmap rings without regenerating or
/// uploading overlapping tile-sized geometry.
pub const SURFACE_PATCH_EDGE_CELLS: i32 = 8;
pub const SURFACE_PATCHES_PER_TILE_EDGE: i32 = SURFACE_TILE_EDGE_CELLS / SURFACE_PATCH_EDGE_CELLS;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[repr(u8)]
pub enum SurfaceLodLevel {
    Stride2 = 0,
    Stride4 = 1,
    Stride8 = 2,
    Stride16 = 3,
}

impl SurfaceLodLevel {
    pub const ALL: [Self; 4] = [Self::Stride2, Self::Stride4, Self::Stride8, Self::Stride16];

    pub const fn index(self) -> u8 {
        self as u8
    }

    pub const fn stride_voxels(self) -> i32 {
        match self {
            Self::Stride2 => 2,
            Self::Stride4 => 4,
            Self::Stride8 => 8,
            Self::Stride16 => 16,
        }
    }

    pub const fn tile_span_voxels(self) -> i32 {
        self.stride_voxels() * SURFACE_TILE_EDGE_CELLS
    }

    pub const fn from_stride_voxels(stride: i32) -> Option<Self> {
        match stride {
            2 => Some(Self::Stride2),
            4 => Some(Self::Stride4),
            8 => Some(Self::Stride8),
            16 => Some(Self::Stride16),
            _ => None,
        }
    }
}

/// A stable streamed-tile key. The LOD level is part of the identity even when two tiles share
/// the same integer X/Z coordinates.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SurfaceTileCoord {
    pub level: SurfaceLodLevel,
    pub x: i32,
    pub z: i32,
}

impl SurfaceTileCoord {
    pub const fn new(level: SurfaceLodLevel, x: i32, z: i32) -> Self {
        Self { level, x, z }
    }

    pub fn containing(level: SurfaceLodLevel, voxel_x: i32, voxel_z: i32) -> Self {
        let span = level.tile_span_voxels();
        Self::new(level, voxel_x.div_euclid(span), voxel_z.div_euclid(span))
    }

    pub const fn stride_voxels(self) -> i32 {
        self.level.stride_voxels()
    }

    pub const fn voxel_span(self) -> i32 {
        self.level.tile_span_voxels()
    }

    pub const fn voxel_origin(self) -> [i32; 2] {
        let span = self.voxel_span();
        [self.x * span, self.z * span]
    }

    /// Horizontal half-open bounds in canonical 10 cm voxel coordinates.
    pub const fn voxel_bounds_xz(self) -> [[i32; 2]; 2] {
        let origin = self.voxel_origin();
        let span = self.voxel_span();
        [origin, [origin[0] + span, origin[1] + span]]
    }
}

/// The original far renderer used the stride-8 member of the generalized LOD family. Keep this
/// small wrapper while shell and renderer migrate to level-aware keys.
pub const FAR_STRIDE_VOXELS: i32 = SurfaceLodLevel::Stride8.stride_voxels();
pub const FAR_TILE_EDGE_CELLS: i32 = SURFACE_TILE_EDGE_CELLS;
pub const FAR_TILE_SPAN_VOXELS: i32 = SurfaceLodLevel::Stride8.tile_span_voxels();

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct FarTileCoord {
    pub x: i32,
    pub z: i32,
}

impl FarTileCoord {
    pub const fn new(x: i32, z: i32) -> Self {
        Self { x, z }
    }

    pub const fn voxel_origin(self) -> [i32; 2] {
        [self.x * FAR_TILE_SPAN_VOXELS, self.z * FAR_TILE_SPAN_VOXELS]
    }

    pub const fn surface_coord(self) -> SurfaceTileCoord {
        SurfaceTileCoord::new(SurfaceLodLevel::Stride8, self.x, self.z)
    }
}

impl From<FarTileCoord> for SurfaceTileCoord {
    fn from(coord: FarTileCoord) -> Self {
        coord.surface_coord()
    }
}

impl TryFrom<SurfaceTileCoord> for FarTileCoord {
    type Error = SurfaceLodLevel;

    fn try_from(coord: SurfaceTileCoord) -> Result<Self, Self::Error> {
        if coord.level == SurfaceLodLevel::Stride8 {
            Ok(Self::new(coord.x, coord.z))
        } else {
            Err(coord.level)
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SurfaceQuad {
    pub origin: [i32; 3],
    pub face: u8,
    pub extent: [u16; 2],
    pub material: Material,
}

/// Conservative half-open voxel bounds derived from actual surface geometry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SurfaceBounds {
    pub min: [i32; 3],
    pub max: [i32; 3],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum SurfacePatchEdge {
    NegativeX,
    PositiveX,
    NegativeZ,
    PositiveZ,
}

impl SurfacePatchEdge {
    pub const ALL: [Self; 4] = [
        Self::NegativeX,
        Self::PositiveX,
        Self::NegativeZ,
        Self::PositiveZ,
    ];

    pub const fn index(self) -> usize {
        self as usize
    }
}

/// A contiguous part of a [`SurfaceTileMesh`]. Cell bounds are local to the tile and half-open;
/// geometry bounds are in canonical 10 cm voxel coordinates and conservatively enclose every quad
/// in `quad_range`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SurfacePatch {
    pub cell_bounds: [[u8; 2]; 2],
    /// Main surface shell. Transition skirts are kept separately so the renderer can enable them
    /// only where this patch touches another geometric LOD owner.
    pub quad_range: Range<u32>,
    pub skirt_ranges: [Range<u32>; 4],
    pub bounds: SurfaceBounds,
}

impl SurfacePatch {
    pub fn quads<'a>(&self, tile: &'a SurfaceTileMesh) -> &'a [SurfaceQuad] {
        &tile.quads[self.quad_range.start as usize..self.quad_range.end as usize]
    }

    pub fn skirt_quads<'a>(
        &self,
        tile: &'a SurfaceTileMesh,
        edge: SurfacePatchEdge,
    ) -> &'a [SurfaceQuad] {
        let range = &self.skirt_ranges[edge.index()];
        &tile.quads[range.start as usize..range.end as usize]
    }
}

/// Structured coarse-surface geometry. Patches are ordered in row-major patch coordinates and
/// each owns one non-overlapping 8x8-cell region of the 32x32-cell tile.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SurfaceTileMesh {
    pub coord: SurfaceTileCoord,
    pub quads: Vec<SurfaceQuad>,
    pub patches: Vec<SurfacePatch>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WaterPatch {
    pub cell_bounds: [[u8; 2]; 2],
    pub quad_range: Range<u32>,
    pub bounds: SurfaceBounds,
}

impl WaterPatch {
    pub fn quads<'a>(&self, tile: &'a WaterTileMesh) -> &'a [SurfaceQuad] {
        &tile.quads[self.quad_range.start as usize..self.quad_range.end as usize]
    }
}

/// Flat, edit-aware sea geometry kept separate from lowered terrain underlays. Only patches with
/// sampled Water occupancy are present, and every quad stays on the canonical sea-level voxel.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WaterTileMesh {
    pub coord: SurfaceTileCoord,
    pub quads: Vec<SurfaceQuad>,
    pub patches: Vec<WaterPatch>,
}

impl SurfaceBounds {
    pub fn from_quads(quads: &[SurfaceQuad]) -> Option<Self> {
        let first = quads.first()?;
        let (mut min, mut max) = quad_voxel_bounds(*first);
        for quad in &quads[1..] {
            let (quad_min, quad_max) = quad_voxel_bounds(*quad);
            for axis in 0..3 {
                min[axis] = min[axis].min(quad_min[axis]);
                max[axis] = max[axis].max(quad_max[axis]);
            }
        }
        Some(Self { min, max })
    }

    fn empty() -> Self {
        Self {
            min: [i32::MAX; 3],
            max: [i32::MIN; 3],
        }
    }

    fn include_quad(&mut self, quad: SurfaceQuad) {
        let (quad_min, quad_max) = quad_voxel_bounds(quad);
        for axis in 0..3 {
            self.min[axis] = self.min[axis].min(quad_min[axis]);
            self.max[axis] = self.max[axis].max(quad_max[axis]);
        }
    }
}

fn quad_voxel_bounds(quad: SurfaceQuad) -> ([i32; 3], [i32; 3]) {
    let mut max = quad.origin.map(|value| value.saturating_add(1));
    let extent = [i32::from(quad.extent[0]), i32::from(quad.extent[1])];
    match quad.face {
        FACE_POS_X | FACE_NEG_X => {
            max[1] = quad.origin[1].saturating_add(extent[1]);
            max[2] = quad.origin[2].saturating_add(extent[0]);
        }
        FACE_POS_Y | FACE_NEG_Y => {
            max[0] = quad.origin[0].saturating_add(extent[0]);
            max[2] = quad.origin[2].saturating_add(extent[1]);
        }
        FACE_POS_Z | FACE_NEG_Z => {
            max[0] = quad.origin[0].saturating_add(extent[0]);
            max[1] = quad.origin[1].saturating_add(extent[1]);
        }
        _ => {}
    }
    (quad.origin, max)
}

/// Returns all tiles at `level` whose surface or cardinal transition shell can depend on a
/// canonical column. The owning tile is always first. A cardinal neighbor is included when the
/// column lies in the one-cell boundary band sampled by that neighbor.
pub fn surface_tiles_affected_by_column(
    level: SurfaceLodLevel,
    voxel_x: i32,
    voxel_z: i32,
) -> Vec<SurfaceTileCoord> {
    let owner = SurfaceTileCoord::containing(level, voxel_x, voxel_z);
    let [origin_x, origin_z] = owner.voxel_origin();
    let local_x = voxel_x - origin_x;
    let local_z = voxel_z - origin_z;
    let stride = level.stride_voxels();
    let span = level.tile_span_voxels();
    let mut affected = Vec::with_capacity(3);
    affected.push(owner);
    if local_x < stride {
        affected.push(SurfaceTileCoord::new(level, owner.x - 1, owner.z));
    }
    if local_x >= span - stride {
        affected.push(SurfaceTileCoord::new(level, owner.x + 1, owner.z));
    }
    if local_z < stride {
        affected.push(SurfaceTileCoord::new(level, owner.x, owner.z - 1));
    }
    if local_z >= span - stride {
        affected.push(SurfaceTileCoord::new(level, owner.x, owner.z + 1));
    }
    affected
}

/// Returns every derived tile whose terrain shell or anchor-owned skyline proxy can change after
/// one canonical voxel edit. Feature ownership follows the anchor rather than the edited column,
/// which matters when a crown crosses a tile or patch boundary.
pub fn surface_tiles_affected_by_voxel(
    generator: Generator,
    level: SurfaceLodLevel,
    coord: VoxelCoord,
) -> Vec<SurfaceTileCoord> {
    let mut affected = surface_tiles_affected_by_column(level, coord.x, coord.z);
    for feature in generator.skyline_features_at(coord) {
        let Some(feature_material) = feature.material_at(coord) else {
            continue;
        };
        if generator.sample(coord.x, coord.y, coord.z) != feature_material {
            continue;
        }
        let owner = SurfaceTileCoord::containing(level, feature.anchor[0], feature.anchor[2]);
        if !affected.contains(&owner) {
            affected.push(owner);
        }
    }
    affected
}

/// Builds a deterministic, surface-preserving coarse shell. Adjacent tiles sample the same global
/// cell centers, and vertical transition quads close every height discontinuity, so independently
/// streamed tiles cannot expose holes.
pub fn generate_surface_tile(generator: Generator, coord: SurfaceTileCoord) -> Vec<SurfaceQuad> {
    generate_edited_surface_tile(generator, &EditMap::default(), coord)
}

pub fn generate_surface_tile_mesh(
    generator: Generator,
    coord: SurfaceTileCoord,
) -> SurfaceTileMesh {
    generate_edited_surface_tile_mesh(generator, &EditMap::default(), coord)
}

/// Builds the compatibility vector form of an edit-aware surface tile. Transition skirts remain
/// omitted because callers of the legacy API cannot select them per patch edge.
pub fn generate_edited_surface_tile(
    generator: Generator,
    edits: &EditMap,
    coord: SurfaceTileCoord,
) -> Vec<SurfaceQuad> {
    let features = pristine_skyline_features(generator, edits, coord);
    generate_surface_tile_mesh_with_options(
        coord,
        |x, z| edits.surface_sample(generator, x, z),
        false,
        &features,
    )
    .into_surface_quads()
}

/// Builds terrain patches and anchor-owned skyline proxies from the same generator plus sparse
/// edit authority used by canonical chunks.
pub fn generate_edited_surface_tile_mesh(
    generator: Generator,
    edits: &EditMap,
    coord: SurfaceTileCoord,
) -> SurfaceTileMesh {
    let features = pristine_skyline_features(generator, edits, coord);
    generate_surface_tile_mesh_with_options(
        coord,
        |x, z| edits.surface_sample(generator, x, z),
        true,
        &features,
    )
}

pub fn generate_edited_water_tile_mesh(
    generator: Generator,
    edits: &EditMap,
    coord: SurfaceTileCoord,
) -> WaterTileMesh {
    generate_water_tile_mesh_with(coord, |x, z| {
        edits
            .override_at(VoxelCoord::new(x, SEA_LEVEL_VOXELS, z))
            .map_or_else(
                || generator.surface_sample(x, z).water_level == Some(SEA_LEVEL_VOXELS),
                |material| material == Material::Water,
            )
    })
}

pub fn generate_water_tile_mesh_with(
    coord: SurfaceTileCoord,
    water_at: impl Fn(i32, i32) -> bool,
) -> WaterTileMesh {
    let [origin_x, origin_z] = coord.voxel_origin();
    let stride = coord.stride_voxels();
    let mut wet = [[false; SURFACE_TILE_EDGE_CELLS as usize]; SURFACE_TILE_EDGE_CELLS as usize];
    for cell_z in 0..SURFACE_TILE_EDGE_CELLS {
        for cell_x in 0..SURFACE_TILE_EDGE_CELLS {
            wet[cell_z as usize][cell_x as usize] = water_at(
                origin_x + cell_x * stride + stride / 2,
                origin_z + cell_z * stride + stride / 2,
            );
        }
    }

    let mut quads = Vec::new();
    let mut patches = Vec::new();
    for patch_z in 0..SURFACE_PATCHES_PER_TILE_EDGE {
        for patch_x in 0..SURFACE_PATCHES_PER_TILE_EDGE {
            let cell_min_x = patch_x * SURFACE_PATCH_EDGE_CELLS;
            let cell_min_z = patch_z * SURFACE_PATCH_EDGE_CELLS;
            let mut consumed =
                [[false; SURFACE_PATCH_EDGE_CELLS as usize]; SURFACE_PATCH_EDGE_CELLS as usize];
            let quad_start = quads.len() as u32;
            for local_z in 0..SURFACE_PATCH_EDGE_CELLS {
                for local_x in 0..SURFACE_PATCH_EDGE_CELLS {
                    if consumed[local_z as usize][local_x as usize]
                        || !wet[(cell_min_z + local_z) as usize][(cell_min_x + local_x) as usize]
                    {
                        continue;
                    }
                    let mut width = 1;
                    while local_x + width < SURFACE_PATCH_EDGE_CELLS
                        && !consumed[local_z as usize][(local_x + width) as usize]
                        && wet[(cell_min_z + local_z) as usize]
                            [(cell_min_x + local_x + width) as usize]
                    {
                        width += 1;
                    }
                    let mut height = 1;
                    'height: while local_z + height < SURFACE_PATCH_EDGE_CELLS {
                        for offset in 0..width {
                            if consumed[(local_z + height) as usize][(local_x + offset) as usize]
                                || !wet[(cell_min_z + local_z + height) as usize]
                                    [(cell_min_x + local_x + offset) as usize]
                            {
                                break 'height;
                            }
                        }
                        height += 1;
                    }
                    for z in local_z..local_z + height {
                        for x in local_x..local_x + width {
                            consumed[z as usize][x as usize] = true;
                        }
                    }
                    quads.push(SurfaceQuad {
                        origin: [
                            origin_x + (cell_min_x + local_x) * stride,
                            SEA_LEVEL_VOXELS,
                            origin_z + (cell_min_z + local_z) * stride,
                        ],
                        face: FACE_POS_Y,
                        extent: [(width * stride) as u16, (height * stride) as u16],
                        material: Material::Water,
                    });
                }
            }
            let quad_end = quads.len() as u32;
            if quad_start == quad_end {
                continue;
            }
            let bounds = SurfaceBounds::from_quads(&quads[quad_start as usize..quad_end as usize])
                .unwrap_or(SurfaceBounds {
                    min: [origin_x, SEA_LEVEL_VOXELS, origin_z],
                    max: [origin_x, SEA_LEVEL_VOXELS + 1, origin_z],
                });
            patches.push(WaterPatch {
                cell_bounds: [
                    [cell_min_x as u8, cell_min_z as u8],
                    [
                        (cell_min_x + SURFACE_PATCH_EDGE_CELLS) as u8,
                        (cell_min_z + SURFACE_PATCH_EDGE_CELLS) as u8,
                    ],
                ],
                quad_range: quad_start..quad_end,
                bounds,
            });
        }
    }
    WaterTileMesh {
        coord,
        quads,
        patches,
    }
}

pub fn generate_surface_tile_with(
    coord: SurfaceTileCoord,
    surface: impl Fn(i32, i32) -> (i32, Material),
) -> Vec<SurfaceQuad> {
    generate_surface_tile_mesh_with_options(coord, surface, false, &[]).into_surface_quads()
}

impl SurfaceTileMesh {
    /// Flattens only the main shell for compatibility with callers that cannot selectively draw
    /// per-edge transition skirts.
    pub fn into_surface_quads(self) -> Vec<SurfaceQuad> {
        let mut surface = Vec::new();
        for patch in &self.patches {
            surface.extend_from_slice(
                &self.quads[patch.quad_range.start as usize..patch.quad_range.end as usize],
            );
        }
        surface
    }
}

pub fn generate_surface_tile_mesh_with(
    coord: SurfaceTileCoord,
    surface: impl Fn(i32, i32) -> (i32, Material),
) -> SurfaceTileMesh {
    generate_surface_tile_mesh_with_options(coord, surface, true, &[])
}

fn generate_surface_tile_mesh_with_options(
    coord: SurfaceTileCoord,
    surface: impl Fn(i32, i32) -> (i32, Material),
    transition_skirts: bool,
    skyline_features: &[SkylineFeature],
) -> SurfaceTileMesh {
    let [origin_x, origin_z] = coord.voxel_origin();
    let stride = coord.stride_voxels();
    let edge = SURFACE_TILE_EDGE_CELLS;
    let sample_edge = edge + 2;
    let mut samples = Vec::with_capacity((sample_edge * sample_edge) as usize);
    for sample_z in -1..=edge {
        for sample_x in -1..=edge {
            samples.push(surface(
                origin_x + sample_x * stride + stride / 2,
                origin_z + sample_z * stride + stride / 2,
            ));
        }
    }
    let sample = |cell_x: i32, cell_z: i32| {
        let index = (cell_x + 1) + (cell_z + 1) * sample_edge;
        samples[index as usize]
    };

    let mut quads = Vec::with_capacity((edge * edge * 4) as usize);
    let mut patches = Vec::with_capacity(
        (SURFACE_PATCHES_PER_TILE_EDGE * SURFACE_PATCHES_PER_TILE_EDGE) as usize,
    );
    for patch_z in 0..SURFACE_PATCHES_PER_TILE_EDGE {
        for patch_x in 0..SURFACE_PATCHES_PER_TILE_EDGE {
            let cell_min_x = patch_x * SURFACE_PATCH_EDGE_CELLS;
            let cell_min_z = patch_z * SURFACE_PATCH_EDGE_CELLS;
            let quad_start = quads.len() as u32;
            let mut bounds = SurfaceBounds::empty();
            for cell_z in cell_min_z..cell_min_z + SURFACE_PATCH_EDGE_CELLS {
                for cell_x in cell_min_x..cell_min_x + SURFACE_PATCH_EDGE_CELLS {
                    let x = origin_x + cell_x * stride;
                    let z = origin_z + cell_z * stride;
                    let (height, material) = sample(cell_x, cell_z);
                    let top = SurfaceQuad {
                        origin: [x, height, z],
                        face: FACE_POS_Y,
                        extent: [stride as u16; 2],
                        material,
                    };
                    bounds.include_quad(top);
                    quads.push(top);

                    let neighbors = [
                        (-1, 0, FACE_NEG_X),
                        (1, 0, FACE_POS_X),
                        (0, -1, FACE_NEG_Z),
                        (0, 1, FACE_POS_Z),
                    ];
                    for (dx, dz, face) in neighbors {
                        let (neighbor_height, _) = sample(cell_x + dx, cell_z + dz);
                        if height <= neighbor_height {
                            continue;
                        }
                        let side_origin = match face {
                            FACE_POS_X => [x + stride - 1, neighbor_height + 1, z],
                            FACE_NEG_X => [x, neighbor_height + 1, z],
                            FACE_POS_Z => [x, neighbor_height + 1, z + stride - 1],
                            _ => [x, neighbor_height + 1, z],
                        };
                        let side = SurfaceQuad {
                            origin: side_origin,
                            face,
                            extent: [stride as u16, (height - neighbor_height) as u16],
                            material,
                        };
                        bounds.include_quad(side);
                        quads.push(side);
                    }
                }
            }
            let patch_min_x = origin_x + cell_min_x * stride;
            let patch_min_z = origin_z + cell_min_z * stride;
            let patch_max_x = patch_min_x + SURFACE_PATCH_EDGE_CELLS * stride;
            let patch_max_z = patch_min_z + SURFACE_PATCH_EDGE_CELLS * stride;
            for feature in skyline_features.iter().copied().filter(|feature| {
                feature.anchor[0] >= patch_min_x
                    && feature.anchor[0] < patch_max_x
                    && feature.anchor[2] >= patch_min_z
                    && feature.anchor[2] < patch_max_z
            }) {
                let feature_start = quads.len();
                append_skyline_proxy(&mut quads, coord.level, feature);
                for quad in &quads[feature_start..] {
                    bounds.include_quad(*quad);
                }
            }
            let quad_end = quads.len() as u32;
            let quad_range = quad_start..quad_end;
            let skirt_ranges = std::array::from_fn(|_| quad_end..quad_end);
            patches.push(SurfacePatch {
                cell_bounds: [
                    [cell_min_x as u8, cell_min_z as u8],
                    [
                        (cell_min_x + SURFACE_PATCH_EDGE_CELLS) as u8,
                        (cell_min_z + SURFACE_PATCH_EDGE_CELLS) as u8,
                    ],
                ],
                quad_range,
                skirt_ranges,
                bounds,
            });
        }
    }
    if transition_skirts {
        for patch in &mut patches {
            let cell_min_x = i32::from(patch.cell_bounds[0][0]);
            let cell_min_z = i32::from(patch.cell_bounds[0][1]);
            patch.skirt_ranges = std::array::from_fn(|edge_index| {
                let skirt_start = quads.len() as u32;
                let patch_edge = SurfacePatchEdge::ALL[edge_index];
                for edge_cell in 0..SURFACE_PATCH_EDGE_CELLS {
                    let (cell_x, cell_z, face) = match patch_edge {
                        SurfacePatchEdge::NegativeX => {
                            (cell_min_x, cell_min_z + edge_cell, FACE_NEG_X)
                        }
                        SurfacePatchEdge::PositiveX => (
                            cell_min_x + SURFACE_PATCH_EDGE_CELLS - 1,
                            cell_min_z + edge_cell,
                            FACE_POS_X,
                        ),
                        SurfacePatchEdge::NegativeZ => {
                            (cell_min_x + edge_cell, cell_min_z, FACE_NEG_Z)
                        }
                        SurfacePatchEdge::PositiveZ => (
                            cell_min_x + edge_cell,
                            cell_min_z + SURFACE_PATCH_EDGE_CELLS - 1,
                            FACE_POS_Z,
                        ),
                    };
                    let x = origin_x + cell_x * stride;
                    let z = origin_z + cell_z * stride;
                    let (height, material) = sample(cell_x, cell_z);
                    let skirt_depth = 128i32.max(stride * 8);
                    let origin = match patch_edge {
                        SurfacePatchEdge::NegativeX => [x, height - skirt_depth, z],
                        SurfacePatchEdge::PositiveX => [x + stride - 1, height - skirt_depth, z],
                        SurfacePatchEdge::NegativeZ => [x, height - skirt_depth, z],
                        SurfacePatchEdge::PositiveZ => [x, height - skirt_depth, z + stride - 1],
                    };
                    quads.push(SurfaceQuad {
                        origin,
                        face,
                        extent: [stride as u16, skirt_depth as u16],
                        material,
                    });
                }
                skirt_start..quads.len() as u32
            });
        }
    }
    SurfaceTileMesh {
        coord,
        quads,
        patches,
    }
}

fn pristine_skyline_features(
    generator: Generator,
    edits: &EditMap,
    coord: SurfaceTileCoord,
) -> Vec<SkylineFeature> {
    generator
        .skyline_features_anchored_in(coord.voxel_bounds_xz())
        .into_iter()
        .filter(|feature| edits.skyline_feature_is_pristine(generator, *feature))
        .collect()
}

fn append_skyline_proxy(
    quads: &mut Vec<SurfaceQuad>,
    level: SurfaceLodLevel,
    feature: SkylineFeature,
) {
    let [anchor_x, ground, anchor_z] = feature.anchor;
    let top = feature.trunk_top;
    let radius_bonus = i32::from(feature.prominence.min(2));
    match feature.kind {
        SkylineFeatureKind::Broadleaf => {
            append_box(
                quads,
                [anchor_x - 1, ground + 1, anchor_z - 1],
                [anchor_x + 2, top + 1, anchor_z + 2],
                Material::Wood,
            );
            let crown_layers: &[([i32; 2], i32)] = match level {
                SurfaceLodLevel::Stride2 => &[
                    ([top - 9, top - 5], 6),
                    ([top - 5, top], 8),
                    ([top, top + 3], 5),
                ],
                SurfaceLodLevel::Stride4 => &[([top - 8, top - 2], 8), ([top - 2, top + 3], 6)],
                SurfaceLodLevel::Stride8 | SurfaceLodLevel::Stride16 => &[([top - 7, top + 3], 8)],
            };
            for &([min_y, max_y], radius) in crown_layers {
                let radius = (radius + radius_bonus).min(FEATURE_MAX_RADIUS_VOXELS);
                append_box(
                    quads,
                    [anchor_x - radius, min_y, anchor_z - radius],
                    [anchor_x + radius + 1, max_y, anchor_z + radius + 1],
                    Material::Leaves,
                );
            }
        }
        SkylineFeatureKind::MoorTor => {
            let height = top - ground;
            for &(min, max, radius) in &[
                (ground + 1, ground + height / 3 + 1, 6),
                (ground + height / 3, ground + height * 2 / 3 + 1, 5),
                (ground + height * 2 / 3, top + 1, 3),
            ] {
                let radius = (radius + radius_bonus).min(FEATURE_MAX_RADIUS_VOXELS);
                append_box(
                    quads,
                    [anchor_x - radius, min, anchor_z - radius],
                    [anchor_x + radius + 1, max, anchor_z + radius + 1],
                    Material::Limestone,
                );
            }
        }
        SkylineFeatureKind::AlpineNeedle => {
            let height = top - ground;
            for &(min, max, radius, material) in &[
                (ground + 1, ground + height / 2 + 1, 7, Material::Stone),
                (ground + height / 2, top - 4, 4, Material::Stone),
                (top - 5, top + 1, 2, Material::Snow),
            ] {
                let radius = (radius + radius_bonus).min(FEATURE_MAX_RADIUS_VOXELS);
                append_box(
                    quads,
                    [anchor_x - radius, min, anchor_z - radius],
                    [anchor_x + radius + 1, max, anchor_z + radius + 1],
                    material,
                );
            }
        }
        SkylineFeatureKind::BadlandsHoodoo => {
            append_box(
                quads,
                [
                    anchor_x - 2 - radius_bonus,
                    ground + 1,
                    anchor_z - 2 - radius_bonus,
                ],
                [
                    anchor_x + 3 + radius_bonus,
                    top - 7,
                    anchor_z + 3 + radius_bonus,
                ],
                Material::Clay,
            );
            append_box(
                quads,
                [
                    anchor_x - 4 - radius_bonus,
                    top - 8,
                    anchor_z - 4 - radius_bonus,
                ],
                [
                    anchor_x + 5 + radius_bonus,
                    top - 3,
                    anchor_z + 5 + radius_bonus,
                ],
                Material::Clay,
            );
            append_box(
                quads,
                [
                    anchor_x - 6 - radius_bonus,
                    top - 4,
                    anchor_z - 6 - radius_bonus,
                ],
                [
                    anchor_x + 7 + radius_bonus,
                    top + 1,
                    anchor_z + 7 + radius_bonus,
                ],
                Material::RedSand,
            );
        }
        SkylineFeatureKind::DuneArch => {
            let [axis_x, axis_z] = feature.oriented_offset(7, 0);
            let [half_x, half_z] = [axis_x.abs() + 2, axis_z.abs() + 2];
            for direction in [-1, 1] {
                let [offset_x, offset_z] = feature.oriented_offset(direction * 7, 0);
                append_box(
                    quads,
                    [anchor_x + offset_x - 2, ground + 1, anchor_z + offset_z - 2],
                    [anchor_x + offset_x + 3, top + 1, anchor_z + offset_z + 3],
                    Material::Limestone,
                );
            }
            append_box(
                quads,
                [anchor_x - half_x, top - 4, anchor_z - half_z],
                [anchor_x + half_x + 1, top + 1, anchor_z + half_z + 1],
                Material::Limestone,
            );
        }
        SkylineFeatureKind::BasaltColumns => {
            let height = top - ground;
            for &(offset, column_height, radius) in &[
                ([0, 0], height, 2),
                ([-5, -3], height * 3 / 4, 2),
                ([5, -2], height * 5 / 8, 1),
                ([2, 5], height * 7 / 8, 2),
            ] {
                let radius = radius + radius_bonus;
                let [offset_x, offset_z] = feature.oriented_offset(offset[0], offset[1]);
                append_box(
                    quads,
                    [
                        anchor_x + offset_x - radius,
                        ground + 1,
                        anchor_z + offset_z - radius,
                    ],
                    [
                        anchor_x + offset_x + radius + 1,
                        ground + column_height + 1,
                        anchor_z + offset_z + radius + 1,
                    ],
                    Material::Basalt,
                );
            }
        }
        SkylineFeatureKind::PilgrimCairn => {
            let layers = [
                (
                    ground + 1,
                    (ground + 4).min(top + 1),
                    4,
                    Material::Limestone,
                ),
                (ground + 4, (ground + 7).min(top + 1), 3, Material::Stone),
                (ground + 7, top + 1, 2, Material::Limestone),
            ];
            for &(min_y, max_y, radius, material) in &layers {
                if min_y < max_y {
                    append_box(
                        quads,
                        [anchor_x - radius, min_y, anchor_z - radius],
                        [anchor_x + radius + 1, max_y, anchor_z + radius + 1],
                        material,
                    );
                }
            }
        }
        SkylineFeatureKind::RouteWaystone => {
            append_box(
                quads,
                [anchor_x - 3, ground + 1, anchor_z - 3],
                [anchor_x + 4, ground + 4, anchor_z + 4],
                Material::Limestone,
            );
            append_box(
                quads,
                [anchor_x - 1, ground + 3, anchor_z - 1],
                [anchor_x + 2, top - 1, anchor_z + 2],
                Material::Limestone,
            );
            append_box(
                quads,
                [anchor_x - 2, top - 2, anchor_z - 2],
                [anchor_x + 3, top + 1, anchor_z + 3],
                Material::Stone,
            );
        }
        SkylineFeatureKind::RuinedArch => {
            let [axis_x, axis_z] = feature.oriented_offset(7, 0);
            let [half_x, half_z] = [axis_x.abs() + 2, axis_z.abs() + 2];
            for direction in [-1, 1] {
                let [offset_x, offset_z] = feature.oriented_offset(direction * 7, 0);
                append_box(
                    quads,
                    [anchor_x + offset_x - 2, ground + 1, anchor_z + offset_z - 2],
                    [anchor_x + offset_x + 3, top + 1, anchor_z + offset_z + 3],
                    Material::Limestone,
                );
            }
            append_box(
                quads,
                [anchor_x - half_x, top - 3, anchor_z - half_z],
                [anchor_x + half_x + 1, top + 1, anchor_z + half_z + 1],
                Material::Stone,
            );
        }
        SkylineFeatureKind::ElderCanopy => {
            append_box(
                quads,
                [anchor_x - 2, ground + 1, anchor_z - 2],
                [anchor_x + 3, top + 1, anchor_z + 3],
                Material::Wood,
            );
            for &(offset_x, offset_z, radius, min_y) in &[
                (0, 0, 15, top - 10),
                (-7, -4, 9, top - 6),
                (7, 5, 9, top - 5),
            ] {
                append_box(
                    quads,
                    [
                        anchor_x + offset_x - radius,
                        min_y,
                        anchor_z + offset_z - radius,
                    ],
                    [
                        anchor_x + offset_x + radius + 1,
                        top + 6,
                        anchor_z + offset_z + radius + 1,
                    ],
                    Material::Leaves,
                );
            }
        }
        SkylineFeatureKind::TorCircle => {
            let height = top - ground;
            for &(offset, column_height) in &[
                ([0, 0], height),
                ([-11, 0], height * 3 / 4),
                ([11, 0], height * 4 / 5),
                ([0, 11], height * 7 / 8),
            ] {
                let [offset_x, offset_z] = feature.oriented_offset(offset[0], offset[1]);
                append_box(
                    quads,
                    [anchor_x + offset_x - 3, ground + 1, anchor_z + offset_z - 3],
                    [
                        anchor_x + offset_x + 4,
                        ground + column_height + 1,
                        anchor_z + offset_z + 4,
                    ],
                    Material::Limestone,
                );
            }
        }
        SkylineFeatureKind::NeedleGate => {
            for direction in [-1, 1] {
                let [offset_x, offset_z] = feature.oriented_offset(direction * 10, 0);
                append_box(
                    quads,
                    [anchor_x + offset_x - 3, ground + 1, anchor_z + offset_z - 3],
                    [anchor_x + offset_x + 4, top + 1, anchor_z + offset_z + 4],
                    Material::Stone,
                );
            }
            let [axis_x, axis_z] = feature.oriented_offset(12, 3);
            append_box(
                quads,
                [anchor_x - axis_x.abs(), top - 5, anchor_z - axis_z.abs()],
                [
                    anchor_x + axis_x.abs() + 1,
                    top + 1,
                    anchor_z + axis_z.abs() + 1,
                ],
                Material::Snow,
            );
        }
        SkylineFeatureKind::BuriedRibs => {
            for direction in [-1, 1] {
                let [offset_x, offset_z] = feature.oriented_offset(0, direction * 9);
                append_box(
                    quads,
                    [anchor_x + offset_x - 2, ground + 1, anchor_z + offset_z - 2],
                    [anchor_x + offset_x + 3, top - 5, anchor_z + offset_z + 3],
                    Material::Stone,
                );
            }
            for along in [-7, 7] {
                let [offset_x, offset_z] = feature.oriented_offset(along, 0);
                let [axis_x, axis_z] = feature.oriented_offset(2, 10);
                append_box(
                    quads,
                    [
                        anchor_x + offset_x - axis_x.abs(),
                        top - 7,
                        anchor_z + offset_z - axis_z.abs(),
                    ],
                    [
                        anchor_x + offset_x + axis_x.abs() + 1,
                        top + 1,
                        anchor_z + offset_z + axis_z.abs() + 1,
                    ],
                    Material::Limestone,
                );
            }
        }
        SkylineFeatureKind::BuriedColonnade => {
            for along in [-10, 0, 10] {
                let [offset_x, offset_z] = feature.oriented_offset(along, 0);
                append_box(
                    quads,
                    [anchor_x + offset_x - 2, ground + 1, anchor_z + offset_z - 2],
                    [anchor_x + offset_x + 3, top + 1, anchor_z + offset_z + 3],
                    Material::Limestone,
                );
            }
            let [axis_x, axis_z] = feature.oriented_offset(14, 3);
            append_box(
                quads,
                [anchor_x - axis_x.abs(), top - 4, anchor_z - axis_z.abs()],
                [
                    anchor_x + axis_x.abs() + 1,
                    top + 1,
                    anchor_z + axis_z.abs() + 1,
                ],
                Material::Stone,
            );
        }
        SkylineFeatureKind::BasaltCrown => {
            let height = top - ground;
            for &(offset, column_height) in &[
                ([0, 0], height),
                ([-10, -5], height * 4 / 5),
                ([10, -5], height * 3 / 4),
                ([0, 12], height * 3 / 5),
            ] {
                let [offset_x, offset_z] = feature.oriented_offset(offset[0], offset[1]);
                append_box(
                    quads,
                    [anchor_x + offset_x - 3, ground + 1, anchor_z + offset_z - 3],
                    [
                        anchor_x + offset_x + 4,
                        ground + column_height + 1,
                        anchor_z + offset_z + 4,
                    ],
                    Material::Basalt,
                );
            }
        }
    }
}

fn append_box(quads: &mut Vec<SurfaceQuad>, min: [i32; 3], max: [i32; 3], material: Material) {
    let size = [max[0] - min[0], max[1] - min[1], max[2] - min[2]];
    debug_assert!(
        size.into_iter()
            .all(|size| size > 0 && size <= i32::from(u16::MAX))
    );
    let size = size.map(|size| size as u16);
    quads.extend([
        SurfaceQuad {
            origin: [max[0] - 1, min[1], min[2]],
            face: FACE_POS_X,
            extent: [size[2], size[1]],
            material,
        },
        SurfaceQuad {
            origin: min,
            face: FACE_NEG_X,
            extent: [size[2], size[1]],
            material,
        },
        SurfaceQuad {
            origin: [min[0], max[1] - 1, min[2]],
            face: FACE_POS_Y,
            extent: [size[0], size[2]],
            material,
        },
        SurfaceQuad {
            origin: min,
            face: FACE_NEG_Y,
            extent: [size[0], size[2]],
            material,
        },
        SurfaceQuad {
            origin: [min[0], min[1], max[2] - 1],
            face: FACE_POS_Z,
            extent: [size[0], size[1]],
            material,
        },
        SurfaceQuad {
            origin: min,
            face: FACE_NEG_Z,
            extent: [size[0], size[1]],
            material,
        },
    ]);
}

pub fn generate_far_tile(generator: Generator, coord: FarTileCoord) -> Vec<SurfaceQuad> {
    generate_surface_tile(generator, coord.surface_coord())
}

pub fn generate_far_tile_with(
    coord: FarTileCoord,
    surface: impl Fn(i32, i32) -> (i32, Material),
) -> Vec<SurfaceQuad> {
    generate_surface_tile_with(coord.surface_coord(), surface)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{BTreeSet, VecDeque};

    fn nearby_skyline_feature(generator: Generator) -> SkylineFeature {
        generator
            .skyline_features_anchored_in([[-1_024, -1_024], [1_024, 1_024]])
            .into_iter()
            .find(|feature| feature.kind == SkylineFeatureKind::Broadleaf)
            .expect("fixed seed should place a nearby skyline feature")
    }

    fn proxy_quads_per_feature(level: SurfaceLodLevel) -> usize {
        match level {
            SurfaceLodLevel::Stride2 => 24,
            SurfaceLodLevel::Stride4 => 18,
            SurfaceLodLevel::Stride8 | SurfaceLodLevel::Stride16 => 12,
        }
    }

    fn skyline_quad_count(mesh: &SurfaceTileMesh) -> usize {
        mesh.patches
            .iter()
            .flat_map(|patch| patch.quads(mesh))
            .filter(|quad| matches!(quad.material, Material::Wood | Material::Leaves))
            .count()
    }

    #[test]
    fn every_landmark_proxy_is_patch_packed_and_bounded_to_four_boxes() {
        for (index, kind) in SkylineFeatureKind::ALL.into_iter().enumerate() {
            let feature = SkylineFeature {
                id: crate::SkylineFeatureId {
                    cell_x: 0,
                    cell_z: 0,
                },
                kind,
                anchor: [32, 20, 32],
                trunk_top: 60,
                orientation: index as u8,
                variant: (index % 4) as u8,
                prominence: (index % 3) as u8,
                route_landmark: None,
            };
            assert!(feature.material_at(VoxelCoord::new(32, 60, 32)).is_some());
            for level in SurfaceLodLevel::ALL {
                let mut quads = Vec::new();
                append_skyline_proxy(&mut quads, level, feature);
                assert!(!quads.is_empty());
                assert!(quads.len() <= 24, "{kind:?} emitted {} quads", quads.len());
                assert!(quads.iter().all(|quad| quad.material.is_renderable()));
            }
        }
    }

    #[test]
    fn semantic_hero_proxies_are_edit_suppressed_and_exactly_restored() {
        let generator = Generator::new(0x5eed_cafe);
        for kind in SkylineFeatureKind::REGIONAL_HEROES {
            let feature = generator
                .nearest_prominent_skyline_feature(0, 0, kind, 192)
                .expect("fixed catalog should contain every semantic hero");
            let target = VoxelCoord::new(feature.anchor[0], feature.trunk_top, feature.anchor[2]);
            let generated = feature
                .material_at(target)
                .expect("hero inspection probe must be canonical geometry");
            assert_eq!(generator.sample(target.x, target.y, target.z), generated);
            for level in SurfaceLodLevel::ALL {
                let coord =
                    SurfaceTileCoord::containing(level, feature.anchor[0], feature.anchor[2]);
                let pristine =
                    generate_edited_surface_tile_mesh(generator, &EditMap::default(), coord);
                let mut direct_proxy = Vec::new();
                append_skyline_proxy(&mut direct_proxy, level, feature);
                assert!(!direct_proxy.is_empty());
                assert!(direct_proxy.len() <= 24);

                let mut edits = EditMap::default();
                edits.set(generator, target, Material::Air);
                let edited = generate_edited_surface_tile_mesh(generator, &edits, coord);
                assert_eq!(
                    pristine.quads.len() - edited.quads.len(),
                    direct_proxy.len(),
                    "{kind:?} proxy did not disappear at {level:?}"
                );
                edits.set(generator, target, generated);
                assert!(edits.is_empty());
                assert_eq!(
                    generate_edited_surface_tile_mesh(generator, &edits, coord),
                    pristine,
                    "{kind:?} proxy did not restore exactly at {level:?}"
                );
            }
        }
    }

    fn water_quad_covers(tile: &WaterTileMesh, voxel_x: i32, voxel_z: i32) -> bool {
        tile.quads.iter().any(|quad| {
            quad.origin[0] <= voxel_x
                && voxel_x < quad.origin[0] + i32::from(quad.extent[0])
                && quad.origin[2] <= voxel_z
                && voxel_z < quad.origin[2] + i32::from(quad.extent[1])
        })
    }

    #[test]
    fn levels_have_explicit_power_of_two_strides_and_spans() {
        let strides: Vec<_> = SurfaceLodLevel::ALL
            .into_iter()
            .map(SurfaceLodLevel::stride_voxels)
            .collect();
        let spans: Vec<_> = SurfaceLodLevel::ALL
            .into_iter()
            .map(SurfaceLodLevel::tile_span_voxels)
            .collect();
        assert_eq!(strides, [2, 4, 8, 16]);
        assert_eq!(spans, [64, 128, 256, 512]);
        assert_eq!(
            SurfaceLodLevel::from_stride_voxels(8),
            Some(SurfaceLodLevel::Stride8)
        );
        assert_eq!(SurfaceLodLevel::from_stride_voxels(3), None);
    }

    #[test]
    fn level_is_part_of_tile_identity() {
        let tiles = SurfaceLodLevel::ALL
            .map(|level| SurfaceTileCoord::new(level, 3, -2))
            .into_iter()
            .collect::<BTreeSet<_>>();
        assert_eq!(tiles.len(), SurfaceLodLevel::ALL.len());
        assert_eq!(tiles.first().unwrap().voxel_origin(), [192, -128]);
        assert_eq!(tiles.last().unwrap().voxel_origin(), [1536, -1024]);
    }

    #[test]
    fn tile_coordinates_use_euclidean_negative_boundaries() {
        let level = SurfaceLodLevel::Stride4;
        assert_eq!(
            SurfaceTileCoord::containing(level, -1, -129),
            SurfaceTileCoord::new(level, -1, -2)
        );
        assert_eq!(
            SurfaceTileCoord::containing(level, -128, 127),
            SurfaceTileCoord::new(level, -1, 0)
        );
        assert_eq!(
            SurfaceTileCoord::new(level, -1, 2).voxel_bounds_xz(),
            [[-128, 256], [0, 384]]
        );
    }

    #[test]
    fn affected_tiles_include_cardinal_boundary_readers() {
        let level = SurfaceLodLevel::Stride8;
        assert_eq!(
            surface_tiles_affected_by_column(level, -1, -1),
            [
                SurfaceTileCoord::new(level, -1, -1),
                SurfaceTileCoord::new(level, 0, -1),
                SurfaceTileCoord::new(level, -1, 0),
            ]
        );
        assert_eq!(
            surface_tiles_affected_by_column(level, -256, -256),
            [
                SurfaceTileCoord::new(level, -1, -1),
                SurfaceTileCoord::new(level, -2, -1),
                SurfaceTileCoord::new(level, -1, -2),
            ]
        );
        assert_eq!(
            surface_tiles_affected_by_column(level, -128, -128),
            [SurfaceTileCoord::new(level, -1, -1)]
        );
    }

    #[test]
    fn full_water_tiles_merge_once_per_patch_at_one_canonical_level() {
        for level in SurfaceLodLevel::ALL {
            let coord = SurfaceTileCoord::new(level, -2, 3);
            let tile = generate_water_tile_mesh_with(coord, |_, _| true);
            assert_eq!(tile.patches.len(), 16);
            assert_eq!(tile.quads.len(), 16);
            assert!(
                tile.patches
                    .iter()
                    .all(|patch| patch.quads(&tile).len() == 1)
            );
            assert!(tile.quads.iter().all(|quad| {
                quad.origin[1] == SEA_LEVEL_VOXELS
                    && quad.face == FACE_POS_Y
                    && quad.material == Material::Water
                    && quad.extent == [(SURFACE_PATCH_EDGE_CELLS * level.stride_voxels()) as u16; 2]
            }));
        }
    }

    #[test]
    fn sparse_water_edits_remove_sampled_lod_occupancy() {
        let generator = Generator::new(0x5eed_cafe);
        let mut target = None;
        'search: for z in (-12_000..=12_000).step_by(37) {
            for x in (-12_000..=12_000).step_by(37) {
                if generator.sample(x, SEA_LEVEL_VOXELS, z) == Material::Water {
                    target = Some(VoxelCoord::new(x, SEA_LEVEL_VOXELS, z));
                    break 'search;
                }
            }
        }
        let Some(target) = target else {
            panic!("fixed seed should expose ocean Water");
        };
        let level = SurfaceLodLevel::Stride2;
        let coord = SurfaceTileCoord::containing(level, target.x, target.z);
        let [origin_x, origin_z] = coord.voxel_origin();
        let stride = level.stride_voxels();
        let sample_x = origin_x + (target.x - origin_x).div_euclid(stride) * stride + stride / 2;
        let sample_z = origin_z + (target.z - origin_z).div_euclid(stride) * stride + stride / 2;
        let sampled = VoxelCoord::new(sample_x, SEA_LEVEL_VOXELS, sample_z);
        assert_eq!(
            generator.sample(sampled.x, sampled.y, sampled.z),
            Material::Water
        );

        let mut edits = EditMap::default();
        let pristine = generate_edited_water_tile_mesh(generator, &edits, coord);
        assert!(water_quad_covers(&pristine, sample_x, sample_z));
        edits.set(generator, sampled, Material::Air);
        let edited = generate_edited_water_tile_mesh(generator, &edits, coord);
        assert!(!water_quad_covers(&edited, sample_x, sample_z));
        edits.set(generator, sampled, Material::Water);
        assert!(water_quad_covers(
            &generate_edited_water_tile_mesh(generator, &edits, coord),
            sample_x,
            sample_z
        ));
        assert!(edits.is_empty());
    }

    #[test]
    fn every_level_covers_exactly_its_canonical_span() {
        for level in SurfaceLodLevel::ALL {
            let coord = SurfaceTileCoord::new(level, -1, 2);
            let tile = generate_surface_tile_with(coord, |x, z| {
                (x.div_euclid(31) + z.div_euclid(47), Material::Stone)
            });
            assert_eq!(
                tile.iter().filter(|quad| quad.face == FACE_POS_Y).count(),
                (SURFACE_TILE_EDGE_CELLS * SURFACE_TILE_EDGE_CELLS) as usize
            );
            let bounds = SurfaceBounds::from_quads(&tile).unwrap();
            let [origin_x, origin_z] = coord.voxel_origin();
            assert_eq!(bounds.min[0], origin_x);
            assert_eq!(bounds.max[0], origin_x + coord.voxel_span());
            assert_eq!(bounds.min[2], origin_z);
            assert_eq!(bounds.max[2], origin_z + coord.voxel_span());
        }
    }

    #[test]
    fn surface_patches_cover_every_cell_once() {
        assert_eq!(SURFACE_PATCH_EDGE_CELLS, 8);
        assert_eq!(SURFACE_PATCHES_PER_TILE_EDGE, 4);

        for level in SurfaceLodLevel::ALL {
            let mesh =
                generate_surface_tile_mesh_with(SurfaceTileCoord::new(level, 2, -3), |x, z| {
                    (x.div_euclid(23) - z.div_euclid(41), Material::Stone)
                });
            assert_eq!(
                mesh.patches.len(),
                (SURFACE_PATCHES_PER_TILE_EDGE * SURFACE_PATCHES_PER_TILE_EDGE) as usize
            );

            let stride = level.stride_voxels();
            let [origin_x, origin_z] = mesh.coord.voxel_origin();
            let mut covered =
                [[false; SURFACE_TILE_EDGE_CELLS as usize]; SURFACE_TILE_EDGE_CELLS as usize];
            for patch in &mesh.patches {
                let [[min_x, min_z], [max_x, max_z]] = patch.cell_bounds;
                assert_eq!(i32::from(max_x - min_x), SURFACE_PATCH_EDGE_CELLS);
                assert_eq!(i32::from(max_z - min_z), SURFACE_PATCH_EDGE_CELLS);

                let mut top_count = 0;
                for quad in patch.quads(&mesh) {
                    if quad.face != FACE_POS_Y {
                        continue;
                    }
                    let cell_x = (quad.origin[0] - origin_x).div_euclid(stride);
                    let cell_z = (quad.origin[2] - origin_z).div_euclid(stride);
                    assert!((i32::from(min_x)..i32::from(max_x)).contains(&cell_x));
                    assert!((i32::from(min_z)..i32::from(max_z)).contains(&cell_z));
                    assert!(!std::mem::replace(
                        &mut covered[cell_z as usize][cell_x as usize],
                        true
                    ));
                    top_count += 1;
                }
                assert_eq!(top_count, (SURFACE_PATCH_EDGE_CELLS.pow(2)) as usize);
            }
            assert!(covered.into_iter().flatten().all(|covered| covered));
        }
    }

    #[test]
    fn surface_patch_ranges_are_contiguous_bounded_and_conservative() {
        let mesh = generate_surface_tile_mesh_with(
            SurfaceTileCoord::new(SurfaceLodLevel::Stride4, -2, 1),
            |x, z| (x.rem_euclid(37) - z.rem_euclid(19), Material::Dirt),
        );
        let mut next_start = 0;
        for patch in &mesh.patches {
            assert_eq!(patch.quad_range.start, next_start);
            assert!(patch.quad_range.start < patch.quad_range.end);
            assert!(patch.quad_range.end as usize <= mesh.quads.len());
            assert_eq!(
                Some(patch.bounds),
                SurfaceBounds::from_quads(patch.quads(&mesh))
            );
            for quad in patch.quads(&mesh) {
                let (quad_min, quad_max) = quad_voxel_bounds(*quad);
                for axis in 0..3 {
                    assert!(patch.bounds.min[axis] <= quad_min[axis]);
                    assert!(patch.bounds.max[axis] >= quad_max[axis]);
                }
            }
            next_start = patch.quad_range.end;
        }
        // Main patch shells are packed before optional transition skirts so the renderer can
        // coalesce the overwhelmingly common non-skirt path into large instanced draws.
        for patch in &mesh.patches {
            for edge in SurfacePatchEdge::ALL {
                let range = &patch.skirt_ranges[edge.index()];
                assert_eq!(range.start, next_start);
                assert_eq!(range.end - range.start, SURFACE_PATCH_EDGE_CELLS as u32);
                let skirts = patch.skirt_quads(&mesh, edge);
                assert_eq!(skirts.len(), SURFACE_PATCH_EDGE_CELLS as usize);
                assert!(skirts.iter().all(|quad| match edge {
                    SurfacePatchEdge::NegativeX => quad.face == FACE_NEG_X,
                    SurfacePatchEdge::PositiveX => quad.face == FACE_POS_X,
                    SurfacePatchEdge::NegativeZ => quad.face == FACE_NEG_Z,
                    SurfacePatchEdge::PositiveZ => quad.face == FACE_POS_Z,
                }));
                next_start = range.end;
            }
        }
        assert_eq!(next_start as usize, mesh.quads.len());
    }

    #[test]
    fn negative_tile_patch_bounds_follow_half_open_cell_bounds() {
        for level in SurfaceLodLevel::ALL {
            let coord = SurfaceTileCoord::new(level, -3, -2);
            let mesh = generate_surface_tile_mesh_with(coord, |x, z| {
                (x.div_euclid(11) + z.div_euclid(13), Material::Grass)
            });
            let [origin_x, origin_z] = coord.voxel_origin();
            let stride = level.stride_voxels();
            for patch in &mesh.patches {
                let [[min_x, min_z], [max_x, max_z]] = patch.cell_bounds;
                assert_eq!(patch.bounds.min[0], origin_x + i32::from(min_x) * stride);
                assert_eq!(patch.bounds.max[0], origin_x + i32::from(max_x) * stride);
                assert_eq!(patch.bounds.min[2], origin_z + i32::from(min_z) * stride);
                assert_eq!(patch.bounds.max[2], origin_z + i32::from(max_z) * stride);
            }
        }
    }

    #[test]
    fn vector_surface_apis_preserve_structured_quad_order() {
        let coord = SurfaceTileCoord::new(SurfaceLodLevel::Stride8, -1, 2);
        let surface = |x: i32, z: i32| (x.div_euclid(17) - z.div_euclid(29), Material::Grass);
        assert_eq!(
            generate_surface_tile_with(coord, surface),
            generate_surface_tile_mesh_with(coord, surface).into_surface_quads()
        );
        assert_eq!(
            generate_surface_tile(Generator::new(42), coord),
            generate_surface_tile_mesh(Generator::new(42), coord).into_surface_quads()
        );
    }

    #[test]
    fn pilgrim_road_is_cardinal_connected_on_every_lod_sampling_lattice() {
        let generator = Generator::new(0x5eed_cafe);
        for level in SurfaceLodLevel::ALL {
            let stride = level.stride_voxels();
            let half = stride / 2;
            let mut core = BTreeSet::new();
            let reach = crate::ROUTE_CORE_HALF_WIDTH_VOXELS.ceil() as i32 + stride;
            for pair in crate::FIRST_PILGRIM_ROAD_NODES.windows(2) {
                let min_x = pair[0].x.min(pair[1].x) - reach;
                let max_x = pair[0].x.max(pair[1].x) + reach;
                let min_z = pair[0].z.min(pair[1].z) - reach;
                let max_z = pair[0].z.max(pair[1].z) + reach;
                let min_cell_x = (min_x - half).div_euclid(stride) - 1;
                let max_cell_x = (max_x - half).div_euclid(stride) + 1;
                let min_cell_z = (min_z - half).div_euclid(stride) - 1;
                let max_cell_z = (max_z - half).div_euclid(stride) + 1;
                for cell_z in min_cell_z..=max_cell_z {
                    for cell_x in min_cell_x..=max_cell_x {
                        let x = cell_x * stride + half;
                        let z = cell_z * stride + half;
                        if crate::sample_first_pilgrim_road(x, z)
                            .is_some_and(|route| route.core > 0.02)
                            && core.insert((cell_x, cell_z))
                        {
                            let sample = generator.surface_sample(x, z);
                            assert!(matches!(
                                sample.material,
                                Material::Limestone | Material::Stone
                            ));
                            assert!(sample.water_level.is_none());
                        }
                    }
                }
            }
            assert!(!core.is_empty());
            let first = *core.first().unwrap();
            let mut queue = VecDeque::from([first]);
            let mut visited = BTreeSet::from([first]);
            while let Some((cell_x, cell_z)) = queue.pop_front() {
                for neighbor in [
                    (cell_x - 1, cell_z),
                    (cell_x + 1, cell_z),
                    (cell_x, cell_z - 1),
                    (cell_x, cell_z + 1),
                ] {
                    if core.contains(&neighbor) && visited.insert(neighbor) {
                        queue.push_back(neighbor);
                    }
                }
            }
            assert_eq!(
                visited.len(),
                core.len(),
                "{level:?} route core split into multiple sampled components"
            );

            // Exercise the actual structured mesh path at one route lattice cell per level.
            let center = [first.0 * stride + half, first.1 * stride + half];
            let coord = SurfaceTileCoord::containing(level, center[0], center[1]);
            let mesh = generate_surface_tile_mesh(generator, coord);
            let cell_origin = [center[0] - half, center[1] - half];
            let expected = generator.surface_sample(center[0], center[1]);
            assert!(mesh.quads.iter().any(|quad| {
                quad.face == FACE_POS_Y
                    && quad.origin[0] == cell_origin[0]
                    && quad.origin[2] == cell_origin[1]
                    && quad.origin[1] == expected.height
                    && quad.extent == [stride as u16; 2]
                    && quad.material == expected.material
            }));
        }
    }

    #[test]
    fn independently_generated_neighbors_share_every_boundary() {
        for level in SurfaceLodLevel::ALL {
            let stride = level.stride_voxels();
            let span = level.tile_span_voxels();
            let surface = |x: i32, z: i32| (x.div_euclid(17) - z.div_euclid(29), Material::Grass);
            let left = generate_surface_tile_with(SurfaceTileCoord::new(level, 0, 0), surface);
            let right = generate_surface_tile_with(SurfaceTileCoord::new(level, 1, 0), surface);
            let forward = generate_surface_tile_with(SurfaceTileCoord::new(level, 0, 1), surface);
            let left_edge: Vec<_> = left
                .iter()
                .filter(|quad| quad.face == FACE_POS_Y && quad.origin[0] == span - stride)
                .collect();
            let right_edge: Vec<_> = right
                .iter()
                .filter(|quad| quad.face == FACE_POS_Y && quad.origin[0] == span)
                .collect();
            assert_eq!(left_edge.len(), SURFACE_TILE_EDGE_CELLS as usize);
            assert_eq!(right_edge.len(), SURFACE_TILE_EDGE_CELLS as usize);
            for (index, (left_quad, right_quad)) in left_edge.iter().zip(&right_edge).enumerate() {
                let z = index as i32 * stride + stride / 2;
                assert_eq!(left_quad.origin[1], surface(span - stride / 2, z).0);
                assert_eq!(right_quad.origin[1], surface(span + stride / 2, z).0);
                assert_eq!(left_quad.origin[0] + stride, right_quad.origin[0]);
            }

            let back_edge: Vec<_> = left
                .iter()
                .filter(|quad| quad.face == FACE_POS_Y && quad.origin[2] == span - stride)
                .collect();
            let forward_edge: Vec<_> = forward
                .iter()
                .filter(|quad| quad.face == FACE_POS_Y && quad.origin[2] == span)
                .collect();
            assert_eq!(back_edge.len(), SURFACE_TILE_EDGE_CELLS as usize);
            assert_eq!(forward_edge.len(), SURFACE_TILE_EDGE_CELLS as usize);
            for (index, (back_quad, forward_quad)) in
                back_edge.iter().zip(&forward_edge).enumerate()
            {
                let x = index as i32 * stride + stride / 2;
                assert_eq!(back_quad.origin[1], surface(x, span - stride / 2).0);
                assert_eq!(forward_quad.origin[1], surface(x, span + stride / 2).0);
                assert_eq!(back_quad.origin[2] + stride, forward_quad.origin[2]);
            }
        }
    }

    #[test]
    fn structured_neighbor_patches_share_same_level_edges() {
        for level in SurfaceLodLevel::ALL {
            let stride = level.stride_voxels();
            let span = level.tile_span_voxels();
            let surface = |x: i32, z: i32| (x.div_euclid(17) - z.div_euclid(29), Material::Grass);
            let left =
                generate_surface_tile_mesh_with(SurfaceTileCoord::new(level, -1, -1), surface);
            let right =
                generate_surface_tile_mesh_with(SurfaceTileCoord::new(level, 0, -1), surface);
            let edge_x = 0;
            let mut left_edge: Vec<_> = left
                .quads
                .iter()
                .filter(|quad| quad.face == FACE_POS_Y && quad.origin[0] == edge_x - stride)
                .map(|quad| (quad.origin[2], quad.origin[1]))
                .collect();
            let mut right_edge: Vec<_> = right
                .quads
                .iter()
                .filter(|quad| quad.face == FACE_POS_Y && quad.origin[0] == edge_x)
                .map(|quad| (quad.origin[2], quad.origin[1]))
                .collect();
            left_edge.sort_unstable();
            right_edge.sort_unstable();
            assert_eq!(left_edge.len(), SURFACE_TILE_EDGE_CELLS as usize);
            assert_eq!(right_edge.len(), SURFACE_TILE_EDGE_CELLS as usize);
            for ((left_z, left_y), (right_z, right_y)) in left_edge.iter().zip(&right_edge) {
                assert_eq!(left_z, right_z);
                assert_eq!(*left_y, surface(-stride / 2, *left_z + stride / 2).0);
                assert_eq!(*right_y, surface(stride / 2, *right_z + stride / 2).0);
            }

            assert_eq!(
                left.coord.voxel_origin()[0] + span,
                right.coord.voxel_origin()[0]
            );
            let left_boundary_patches: Vec<_> = left
                .patches
                .iter()
                .filter(|patch| patch.cell_bounds[1][0] == SURFACE_TILE_EDGE_CELLS as u8)
                .collect();
            let right_boundary_patches: Vec<_> = right
                .patches
                .iter()
                .filter(|patch| patch.cell_bounds[0][0] == 0)
                .collect();
            assert_eq!(
                left_boundary_patches.len(),
                SURFACE_PATCHES_PER_TILE_EDGE as usize
            );
            assert_eq!(
                right_boundary_patches.len(),
                SURFACE_PATCHES_PER_TILE_EDGE as usize
            );
        }
    }

    #[test]
    fn skyline_proxies_are_anchor_owned_once_at_every_level() {
        let generator = Generator::new(0x5eed);
        let target = nearby_skyline_feature(generator);
        for level in SurfaceLodLevel::ALL {
            let coord = SurfaceTileCoord::containing(level, target.anchor[0], target.anchor[2]);
            let features = generator.skyline_features_anchored_in(coord.voxel_bounds_xz());
            let mesh = generate_surface_tile_mesh(generator, coord);
            let expected_per_feature = proxy_quads_per_feature(level);
            assert_eq!(
                skyline_quad_count(&mesh),
                features
                    .iter()
                    .filter(|feature| feature.kind == SkylineFeatureKind::Broadleaf)
                    .count()
                    * expected_per_feature
            );

            let [origin_x, origin_z] = coord.voxel_origin();
            for patch in &mesh.patches {
                let [[min_x, min_z], [max_x, max_z]] = patch.cell_bounds;
                let world_min_x = origin_x + i32::from(min_x) * level.stride_voxels();
                let world_min_z = origin_z + i32::from(min_z) * level.stride_voxels();
                let world_max_x = origin_x + i32::from(max_x) * level.stride_voxels();
                let world_max_z = origin_z + i32::from(max_z) * level.stride_voxels();
                let owned = features
                    .iter()
                    .filter(|feature| {
                        feature.kind == SkylineFeatureKind::Broadleaf
                            && feature.anchor[0] >= world_min_x
                            && feature.anchor[0] < world_max_x
                            && feature.anchor[2] >= world_min_z
                            && feature.anchor[2] < world_max_z
                    })
                    .count();
                let actual = patch
                    .quads(&mesh)
                    .iter()
                    .filter(|quad| matches!(quad.material, Material::Wood | Material::Leaves))
                    .count();
                assert_eq!(actual, owned * expected_per_feature);
                assert!(SurfacePatchEdge::ALL.into_iter().all(|edge| {
                    patch
                        .skirt_quads(&mesh, edge)
                        .iter()
                        .all(|quad| !matches!(quad.material, Material::Wood | Material::Leaves))
                }));
            }
        }
    }

    #[test]
    fn protruding_proxy_expands_culling_bounds_without_changing_patch_cells() {
        let generator = Generator::new(0x5eed);
        let feature = nearby_skyline_feature(generator);
        let level = SurfaceLodLevel::Stride2;
        let coord = SurfaceTileCoord::containing(level, feature.anchor[0], feature.anchor[2]);
        let mesh = generate_surface_tile_mesh(generator, coord);
        let [origin_x, origin_z] = coord.voxel_origin();
        let patch = mesh
            .patches
            .iter()
            .find(|patch| {
                let [[min_x, min_z], [max_x, max_z]] = patch.cell_bounds;
                let stride = level.stride_voxels();
                feature.anchor[0] >= origin_x + i32::from(min_x) * stride
                    && feature.anchor[0] < origin_x + i32::from(max_x) * stride
                    && feature.anchor[2] >= origin_z + i32::from(min_z) * stride
                    && feature.anchor[2] < origin_z + i32::from(max_z) * stride
            })
            .unwrap();
        let [[min_x, min_z], [max_x, max_z]] = patch.cell_bounds;
        let fixed_min = [
            origin_x + i32::from(min_x) * level.stride_voxels(),
            origin_z + i32::from(min_z) * level.stride_voxels(),
        ];
        let fixed_max = [
            origin_x + i32::from(max_x) * level.stride_voxels(),
            origin_z + i32::from(max_z) * level.stride_voxels(),
        ];
        assert!(
            patch.bounds.min[0] < fixed_min[0]
                || patch.bounds.max[0] > fixed_max[0]
                || patch.bounds.min[2] < fixed_min[1]
                || patch.bounds.max[2] > fixed_max[1]
        );
        assert_eq!(patch.cell_bounds, [[min_x, min_z], [max_x, max_z]]);
    }

    #[test]
    fn canonical_feature_edit_suppresses_and_reversion_restores_every_proxy() {
        let generator = Generator::new(0x5eed);
        let feature = nearby_skyline_feature(generator);
        let target = VoxelCoord::new(feature.anchor[0], feature.anchor[1] + 1, feature.anchor[2]);
        assert_eq!(
            generator.sample(target.x, target.y, target.z),
            Material::Wood
        );
        let mut edits = EditMap::default();
        for level in SurfaceLodLevel::ALL {
            let coord = SurfaceTileCoord::containing(level, feature.anchor[0], feature.anchor[2]);
            let pristine =
                skyline_quad_count(&generate_edited_surface_tile_mesh(generator, &edits, coord));
            edits.set(generator, target, Material::Air);
            let edited =
                skyline_quad_count(&generate_edited_surface_tile_mesh(generator, &edits, coord));
            assert_eq!(pristine - edited, proxy_quads_per_feature(level));
            edits.set(generator, target, Material::Wood);
            assert_eq!(
                skyline_quad_count(&generate_edited_surface_tile_mesh(generator, &edits, coord,)),
                pristine
            );
        }
        assert!(edits.is_empty());
    }

    #[test]
    fn route_landmark_proxies_are_bounded_edit_aware_and_reversible_at_every_lod() {
        let generator = Generator::new(0x5eed_cafe);
        let features: Vec<_> = (0..crate::first_pilgrim_route_anchor_count())
            .map(|ordinal| {
                let anchor = crate::first_pilgrim_route_anchor(ordinal).unwrap();
                generator
                    .skyline_features_anchored_in([
                        [anchor.anchor[0], anchor.anchor[1]],
                        [anchor.anchor[0] + 1, anchor.anchor[1] + 1],
                    ])
                    .into_iter()
                    .next()
                    .expect("route landmark should be anchor-owned")
            })
            .collect();
        assert_eq!(
            features.len(),
            usize::from(crate::first_pilgrim_route_anchor_count())
        );

        for feature in features {
            let [min, max] = feature.bounds();
            let (target, material) = (min[1]..max[1])
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
                .expect("route landmark should contain visible canonical structure");
            assert_eq!(generator.sample(target.x, target.y, target.z), material);

            for level in SurfaceLodLevel::ALL {
                let mut direct_proxy = Vec::new();
                append_skyline_proxy(&mut direct_proxy, level, feature);
                assert!(!direct_proxy.is_empty());
                assert!(direct_proxy.len() <= 24);

                let coord =
                    SurfaceTileCoord::containing(level, feature.anchor[0], feature.anchor[2]);
                let pristine =
                    generate_edited_surface_tile_mesh(generator, &EditMap::default(), coord);
                let mut edits = EditMap::default();
                edits.set(generator, target, Material::Air);
                let edited = generate_edited_surface_tile_mesh(generator, &edits, coord);
                assert_eq!(
                    pristine.quads.len() - edited.quads.len(),
                    direct_proxy.len(),
                    "{:?} proxy was not suppressed at {level:?}",
                    feature.kind
                );
                assert!(surface_tiles_affected_by_voxel(generator, level, target).contains(&coord));

                edits.set(generator, target, material);
                assert!(edits.is_empty());
                assert_eq!(
                    generate_edited_surface_tile_mesh(generator, &edits, coord),
                    pristine
                );
            }
        }
    }

    #[test]
    fn edit_outside_analytic_feature_does_not_suppress_proxy() {
        let generator = Generator::new(0x5eed);
        let feature = nearby_skyline_feature(generator);
        let target = VoxelCoord::new(
            feature.anchor[0] + 8,
            feature.anchor[1] + 1,
            feature.anchor[2] + 8,
        );
        assert_eq!(feature.material_at(target), None);
        let level = SurfaceLodLevel::Stride2;
        let coord = SurfaceTileCoord::containing(level, feature.anchor[0], feature.anchor[2]);
        let pristine = skyline_quad_count(&generate_surface_tile_mesh(generator, coord));
        let generated = generator.sample(target.x, target.y, target.z);
        let replacement = if generated == Material::Stone {
            Material::Air
        } else {
            Material::Stone
        };
        let mut edits = EditMap::default();
        edits.set(generator, target, replacement);
        assert_eq!(
            skyline_quad_count(&generate_edited_surface_tile_mesh(generator, &edits, coord,)),
            pristine
        );
    }

    #[test]
    fn crown_edit_invalidates_anchor_owner_across_a_tile_boundary() {
        let generator = Generator::new(0x5eed);
        let level = SurfaceLodLevel::Stride2;
        let (feature, target) = generator
            .skyline_features_anchored_in([[-2_048, -2_048], [2_048, 2_048]])
            .into_iter()
            .find_map(|feature| {
                for dx in [-5, 5] {
                    let target = VoxelCoord::new(
                        feature.anchor[0] + dx,
                        feature.trunk_top - 3,
                        feature.anchor[2],
                    );
                    if feature.material_at(target) == Some(Material::Leaves)
                        && generator.sample(target.x, target.y, target.z) == Material::Leaves
                        && SurfaceTileCoord::containing(level, target.x, target.z)
                            != SurfaceTileCoord::containing(
                                level,
                                feature.anchor[0],
                                feature.anchor[2],
                            )
                    {
                        return Some((feature, target));
                    }
                }
                None
            })
            .expect("fixed seed should place a crown across a stride-2 tile boundary");
        let owner = SurfaceTileCoord::containing(level, feature.anchor[0], feature.anchor[2]);
        assert_ne!(
            owner,
            SurfaceTileCoord::containing(level, target.x, target.z)
        );
        assert!(surface_tiles_affected_by_voxel(generator, level, target).contains(&owner));
    }

    #[test]
    fn far_api_remains_the_stride_eight_level() {
        let far = FarTileCoord::new(-1, 2);
        assert_eq!(far.voxel_origin(), [-256, 512]);
        assert_eq!(
            SurfaceTileCoord::from(far),
            SurfaceTileCoord::new(SurfaceLodLevel::Stride8, -1, 2)
        );
        assert_eq!(
            generate_far_tile(Generator::new(7), far),
            generate_surface_tile(Generator::new(7), far.surface_coord())
        );
    }
}
