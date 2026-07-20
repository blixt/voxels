use crate::{
    CHUNK_EDGE, Chunk, ChunkCoord, FEATURE_MAX_RADIUS_VOXELS, Generator, Material, MeshingHalo,
    SkylineFeature, SurfaceTileCoord,
};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

const EDIT_CHUNK_MAGIC: &[u8; 4] = b"VXED";
const EDIT_CHUNK_VERSION: u16 = 1;
const EDIT_CHUNK_HEADER_BYTES: usize = 60;
const EDIT_CHUNK_ENTRY_BYTES: usize = 4;
const EDIT_CHUNK_HASH_DOMAIN: &[u8] = b"voxels-edit-chunk-v1\0";

/// Integer address of one canonical 10 cm voxel.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct VoxelCoord {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

impl VoxelCoord {
    pub const fn new(x: i32, y: i32, z: i32) -> Self {
        Self { x, y, z }
    }

    pub const fn as_array(self) -> [i32; 3] {
        [self.x, self.y, self.z]
    }

    pub fn chunk(self) -> ChunkCoord {
        let edge = CHUNK_EDGE as i32;
        ChunkCoord::new(
            self.x.div_euclid(edge),
            self.y.div_euclid(edge),
            self.z.div_euclid(edge),
        )
    }

    pub fn local(self) -> [usize; 3] {
        let edge = CHUNK_EDGE as i32;
        [
            self.x.rem_euclid(edge) as usize,
            self.y.rem_euclid(edge) as usize,
            self.z.rem_euclid(edge) as usize,
        ]
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EditChunkCodecError {
    Truncated,
    InvalidMagic,
    UnsupportedVersion(u16),
    InvalidHeader(&'static str),
    CoordinateMismatch,
    UnknownMaterial(u16),
    CorruptHash,
}

impl fmt::Display for EditChunkCodecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated => formatter.write_str("truncated VXED edit chunk"),
            Self::InvalidMagic => formatter.write_str("invalid VXED edit chunk magic"),
            Self::UnsupportedVersion(version) => {
                write!(formatter, "unsupported VXED edit chunk version {version}")
            }
            Self::InvalidHeader(reason) => write!(formatter, "invalid VXED header: {reason}"),
            Self::CoordinateMismatch => {
                formatter.write_str("VXED edit chunk coordinate does not match its key")
            }
            Self::UnknownMaterial(id) => write!(formatter, "unknown VXED material id {id}"),
            Self::CorruptHash => formatter.write_str("VXED edit chunk content hash mismatch"),
        }
    }
}

impl std::error::Error for EditChunkCodecError {}

/// Sparse, deterministic overlay on top of procedural terrain. Overrides are owned once by their
/// canonical chunk and packed behind sorted 15-bit local indices. Spatial queries reconstruct
/// world coordinates from those compact owners instead of retaining duplicate global, chunk, and
/// column B-trees for every edited voxel.
#[derive(Clone, Debug, Default)]
pub struct EditMap {
    // X/Z/Y ordering keeps every vertical chunk column contiguous while still supporting bounded
    // horizontal range scans. Exact chunk lookups remain logarithmic.
    chunks: BTreeMap<(i32, i32, i32), EditedChunk>,
    override_count: usize,
}

#[derive(Clone, Debug, Default)]
struct EditedChunk {
    // Sorted with Y contiguous, then Z, then X so all overrides in one surface column form a
    // directly searchable slice.
    overrides: Vec<(u16, Material)>,
}

impl EditedChunk {
    fn get(&self, local: u16) -> Option<Material> {
        self.overrides
            .binary_search_by_key(&local, |&(index, _)| index)
            .ok()
            .map(|index| self.overrides[index].1)
    }

    fn replace(&mut self, local: u16, material: Option<Material>) -> bool {
        match self
            .overrides
            .binary_search_by_key(&local, |&(index, _)| index)
        {
            Ok(index) => {
                let Some(material) = material else {
                    self.overrides.remove(index);
                    return true;
                };
                if self.overrides[index].1 == material {
                    return false;
                }
                self.overrides[index].1 = material;
                false
            }
            Err(index) => {
                let Some(material) = material else {
                    return false;
                };
                self.overrides.insert(index, (local, material));
                true
            }
        }
    }

    fn is_empty(&self) -> bool {
        self.overrides.is_empty()
    }

    fn column(&self, x: usize, z: usize) -> &[(u16, Material)] {
        let start = local_index([x, 0, z]);
        let end = local_index([x, CHUNK_EDGE - 1, z]);
        let first = self.overrides.partition_point(|&(index, _)| index < start);
        let after = self.overrides.partition_point(|&(index, _)| index <= end);
        &self.overrides[first..after]
    }
}

impl EditMap {
    /// Known logical payload. This deliberately excludes allocator/B-tree node overhead while
    /// counting allocated compact chunk entries.
    pub fn logical_bytes(&self) -> usize {
        size_of::<Self>()
            + self.chunks.len() * size_of::<((i32, i32, i32), EditedChunk)>()
            + self
                .chunks
                .values()
                .map(|chunk| chunk.overrides.capacity() * size_of::<(u16, Material)>())
                .sum::<usize>()
    }
    pub fn sample(&self, generator: Generator, coord: VoxelCoord) -> Material {
        self.resolve_generated(coord, generator.sample(coord.x, coord.y, coord.z))
    }

    pub fn set(&mut self, generator: Generator, coord: VoxelCoord, material: Material) {
        self.set_against_generated(coord, material, generator.sample(coord.x, coord.y, coord.z));
    }

    /// Resolves one authoritative source value against the sparse edit overlay. This is the
    /// transport-safe form: callers can supply a value from a chunk, halo, or bounded sample block
    /// without giving the edit journal a concrete generator.
    pub fn resolve_generated(&self, coord: VoxelCoord, generated: Material) -> Material {
        self.override_at(coord).unwrap_or(generated)
    }

    /// Stores only values that differ from an already obtained authoritative source value.
    pub fn set_against_generated(
        &mut self,
        coord: VoxelCoord,
        material: Material,
        generated: Material,
    ) {
        if generated == material {
            self.replace_override(coord, None);
        } else {
            self.replace_override(coord, Some(material));
        }
    }

    /// Used only when hydrating already validated durable rows.
    pub fn insert_override(&mut self, coord: VoxelCoord, material: Material) {
        self.replace_override(coord, Some(material));
    }

    /// Applies one already validated durable row/change notification. `None` means the row was
    /// removed and the generator is authoritative again.
    pub fn replace_durable_override(&mut self, coord: VoxelCoord, material: Option<Material>) {
        self.replace_override(coord, material);
    }

    /// Applies a validated mutation batch by merging each touched chunk once. This keeps metre-scale
    /// tools linear in the existing and changed entries instead of shifting a sorted vector for
    /// every individual voxel.
    pub fn replace_durable_overrides(&mut self, changes: &[(VoxelCoord, Option<Material>)]) {
        let mut chunks = BTreeMap::<(i32, i32, i32), Vec<(u16, Option<Material>)>>::new();
        for &(coord, material) in changes {
            chunks
                .entry(chunk_key(coord.chunk()))
                .or_default()
                .push((local_index(coord.local()), material));
        }
        for (key, mut changes) in chunks {
            changes.sort_unstable_by_key(|&(index, _)| index);
            debug_assert!(
                changes.windows(2).all(|pair| pair[0].0 != pair[1].0),
                "durable edit batch contains duplicate voxel coordinates"
            );
            let previous = self.chunks.remove(&key).unwrap_or_default();
            self.override_count -= previous.overrides.len();
            let merged = merge_chunk_overrides(previous.overrides, &changes);
            self.override_count += merged.len();
            if !merged.is_empty() {
                self.chunks.insert(key, EditedChunk { overrides: merged });
            }
        }
    }

    pub fn override_at(&self, coord: VoxelCoord) -> Option<Material> {
        self.chunks
            .get(&chunk_key(coord.chunk()))
            .and_then(|chunk| chunk.get(local_index(coord.local())))
    }

    pub fn len(&self) -> usize {
        self.override_count
    }

    pub fn is_empty(&self) -> bool {
        self.override_count == 0
    }

    /// Copies only edits that can affect the requested chunk cores or their one-voxel meshing
    /// shells. Native generation workers use this bounded snapshot so unrelated regions never make
    /// a hot chunk request clone the global edit journal.
    pub fn snapshot_for_chunks(&self, coords: &[ChunkCoord]) -> Self {
        let mut changes = Vec::new();
        for coord in coords {
            let requested_origin = coord.world_origin();
            for dz in -1..=1 {
                for dy in -1..=1 {
                    for dx in -1..=1 {
                        let Some(x) = coord.x.checked_add(dx) else {
                            continue;
                        };
                        let Some(y) = coord.y.checked_add(dy) else {
                            continue;
                        };
                        let Some(z) = coord.z.checked_add(dz) else {
                            continue;
                        };
                        let key = (x, z, y);
                        let Some(chunk) = self.chunks.get(&key) else {
                            continue;
                        };
                        let origin = ChunkCoord::new(x, y, z).world_origin();
                        for &(index, material) in &chunk.overrides {
                            let local = local_coord(index);
                            let voxel = VoxelCoord::new(
                                origin[0] + local[0] as i32,
                                origin[1] + local[1] as i32,
                                origin[2] + local[2] as i32,
                            );
                            if voxel.x >= requested_origin[0].saturating_sub(1)
                                && voxel.x <= requested_origin[0].saturating_add(CHUNK_EDGE as i32)
                                && voxel.y >= requested_origin[1].saturating_sub(1)
                                && voxel.y <= requested_origin[1].saturating_add(CHUNK_EDGE as i32)
                                && voxel.z >= requested_origin[2].saturating_sub(1)
                                && voxel.z <= requested_origin[2].saturating_add(CHUNK_EDGE as i32)
                            {
                                changes.push((voxel, Some(material)));
                            }
                        }
                    }
                }
            }
        }
        changes.sort_unstable_by_key(|&(coord, _)| coord);
        changes.dedup_by_key(|change| change.0);
        let mut snapshot = Self::default();
        snapshot.replace_durable_overrides(&changes);
        snapshot
    }

    /// Copies the bounded horizontal edit neighborhood needed by surface tiles, including their
    /// sampling shell and analytic skyline-feature radius.
    pub fn snapshot_for_surface_tiles(&self, coords: &[SurfaceTileCoord]) -> Self {
        let mut changes = Vec::new();
        for coord in coords {
            let [origin_x, origin_z] = coord.voxel_origin();
            let span = i64::from(coord.voxel_span());
            let margin = i64::from(coord.stride_voxels() + FEATURE_MAX_RADIUS_VOXELS);
            let min_x = i64::from(origin_x) - margin;
            let min_z = i64::from(origin_z) - margin;
            let max_x = i64::from(origin_x) + span + margin;
            let max_z = i64::from(origin_z) + span + margin;
            for (voxel, material) in self.overrides_in_horizontal_bounds(min_x, min_z, max_x, max_z)
            {
                changes.push((voxel, Some(material)));
            }
        }
        changes.sort_unstable_by_key(|&(coord, _)| coord);
        changes.dedup_by_key(|change| change.0);
        let mut snapshot = Self::default();
        snapshot.replace_durable_overrides(&changes);
        snapshot
    }

    /// Copies every vertical override in an exact horizontal voxel rectangle. Edit planning uses
    /// this column-complete view because a below-ground mutation can become the new visible surface
    /// after another voxel in the same column is removed.
    pub fn snapshot_for_voxel_columns(
        &self,
        min_x: i32,
        max_x: i32,
        min_z: i32,
        max_z: i32,
    ) -> Self {
        if min_x > max_x || min_z > max_z {
            return Self::default();
        }
        let minimum_chunk = VoxelCoord::new(min_x, 0, min_z).chunk();
        let maximum_chunk = VoxelCoord::new(max_x, 0, max_z).chunk();
        let mut changes = Vec::new();
        for chunk_x in minimum_chunk.x..=maximum_chunk.x {
            for chunk_z in minimum_chunk.z..=maximum_chunk.z {
                for (&key, chunk) in self
                    .chunks
                    .range((chunk_x, chunk_z, i32::MIN)..=(chunk_x, chunk_z, i32::MAX))
                {
                    let origin = ChunkCoord::new(key.0, key.2, key.1).world_origin();
                    for &(index, material) in &chunk.overrides {
                        let local = local_coord(index);
                        let coord = VoxelCoord::new(
                            origin[0] + local[0] as i32,
                            origin[1] + local[1] as i32,
                            origin[2] + local[2] as i32,
                        );
                        if (min_x..=max_x).contains(&coord.x) && (min_z..=max_z).contains(&coord.z)
                        {
                            changes.push((coord, Some(material)));
                        }
                    }
                }
            }
        }
        let mut snapshot = Self::default();
        snapshot.replace_durable_overrides(&changes);
        snapshot
    }

    fn overrides_in_horizontal_bounds(
        &self,
        min_x: i64,
        min_z: i64,
        max_x: i64,
        max_z: i64,
    ) -> impl Iterator<Item = (VoxelCoord, Material)> + '_ {
        let min_chunk_x = min_x
            .div_euclid(CHUNK_EDGE as i64)
            .clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32;
        let max_chunk_x = (max_x - 1)
            .div_euclid(CHUNK_EDGE as i64)
            .clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32;
        self.chunks
            .range((min_chunk_x, i32::MIN, i32::MIN)..=(max_chunk_x, i32::MAX, i32::MAX))
            .flat_map(move |(&(chunk_x, chunk_z, chunk_y), chunk)| {
                let origin = ChunkCoord::new(chunk_x, chunk_y, chunk_z).world_origin();
                chunk
                    .overrides
                    .iter()
                    .copied()
                    .filter_map(move |(index, material)| {
                        let local = local_coord(index);
                        let voxel = VoxelCoord::new(
                            origin[0] + local[0] as i32,
                            origin[1] + local[1] as i32,
                            origin[2] + local[2] as i32,
                        );
                        (i64::from(voxel.x) >= min_x
                            && i64::from(voxel.x) < max_x
                            && i64::from(voxel.z) >= min_z
                            && i64::from(voxel.z) < max_z)
                            .then_some((voxel, material))
                    })
            })
    }

    fn edited_columns_in(&self, bounds: [[i32; 2]; 2]) -> BTreeSet<(i32, i32)> {
        let [[min_x, min_z], [max_x, max_z]] = bounds;
        self.overrides_in_horizontal_bounds(
            i64::from(min_x),
            i64::from(min_z),
            i64::from(max_x),
            i64::from(max_z),
        )
        .map(|(coord, _)| (coord.x, coord.z))
        .collect()
    }

    /// Returns all durable overrides in one canonical chunk in deterministic local-index order.
    pub fn chunk_overrides(&self, coord: ChunkCoord) -> Vec<(VoxelCoord, Material)> {
        let Some(chunk) = self.chunks.get(&chunk_key(coord)) else {
            return Vec::new();
        };
        let origin = coord.world_origin();
        chunk
            .overrides
            .iter()
            .map(|&(index, material)| {
                let local = local_coord(index);
                (
                    VoxelCoord::new(
                        origin[0] + local[0] as i32,
                        origin[1] + local[1] as i32,
                        origin[2] + local[2] as i32,
                    ),
                    material,
                )
            })
            .collect()
    }

    /// Returns the edited canonical chunks in deterministic X/Z/Y order.
    pub fn edited_chunks(&self) -> Vec<ChunkCoord> {
        self.chunks
            .keys()
            .map(|&(x, z, y)| ChunkCoord::new(x, y, z))
            .collect()
    }

    /// Encodes one independently checksummed durable edit chunk. Pristine chunks have no record.
    pub fn encode_chunk_overrides(&self, coord: ChunkCoord) -> Option<Vec<u8>> {
        let chunk = self.chunks.get(&chunk_key(coord))?;
        let count = u32::try_from(chunk.overrides.len()).ok()?;
        let mut entries = Vec::with_capacity(chunk.overrides.len() * EDIT_CHUNK_ENTRY_BYTES);
        for &(index, material) in &chunk.overrides {
            entries.extend_from_slice(&index.to_le_bytes());
            entries.extend_from_slice(&material.id().to_le_bytes());
        }
        let hash = edit_chunk_hash(coord, &entries);
        let mut encoded = Vec::with_capacity(EDIT_CHUNK_HEADER_BYTES + entries.len());
        encoded.extend_from_slice(EDIT_CHUNK_MAGIC);
        encoded.extend_from_slice(&EDIT_CHUNK_VERSION.to_le_bytes());
        encoded.extend_from_slice(&(EDIT_CHUNK_HEADER_BYTES as u16).to_le_bytes());
        encoded.extend_from_slice(&Material::SCHEMA_VERSION.to_le_bytes());
        encoded.extend_from_slice(&0_u16.to_le_bytes());
        encoded.extend_from_slice(&count.to_le_bytes());
        encoded.extend_from_slice(&coord.x.to_le_bytes());
        encoded.extend_from_slice(&coord.y.to_le_bytes());
        encoded.extend_from_slice(&coord.z.to_le_bytes());
        encoded.extend_from_slice(hash.as_bytes());
        debug_assert_eq!(encoded.len(), EDIT_CHUNK_HEADER_BYTES);
        encoded.extend_from_slice(&entries);
        Some(encoded)
    }

    /// Hydrates one durable edit chunk after strict coordinate, ordering, schema, and hash checks.
    pub fn decode_chunk_overrides(
        &mut self,
        coord: ChunkCoord,
        bytes: &[u8],
    ) -> Result<(), EditChunkCodecError> {
        if bytes.len() < 8 {
            return Err(EditChunkCodecError::Truncated);
        }
        if &bytes[..4] != EDIT_CHUNK_MAGIC {
            return Err(EditChunkCodecError::InvalidMagic);
        }
        let version = read_edit_u16(bytes, 4)?;
        if version != EDIT_CHUNK_VERSION {
            return Err(EditChunkCodecError::UnsupportedVersion(version));
        }
        if bytes.len() < EDIT_CHUNK_HEADER_BYTES {
            return Err(EditChunkCodecError::Truncated);
        }
        if usize::from(read_edit_u16(bytes, 6)?) != EDIT_CHUNK_HEADER_BYTES {
            return Err(EditChunkCodecError::InvalidHeader(
                "unexpected header length",
            ));
        }
        if read_edit_u16(bytes, 8)? != Material::SCHEMA_VERSION {
            return Err(EditChunkCodecError::InvalidHeader(
                "material schema mismatch",
            ));
        }
        if read_edit_u16(bytes, 10)? != 0 {
            return Err(EditChunkCodecError::InvalidHeader(
                "reserved field is nonzero",
            ));
        }
        let count = usize::try_from(read_edit_u32(bytes, 12)?)
            .map_err(|_| EditChunkCodecError::InvalidHeader("entry count overflow"))?;
        if count == 0 || count > crate::CHUNK_VOLUME {
            return Err(EditChunkCodecError::InvalidHeader(
                "entry count is out of bounds",
            ));
        }
        let encoded_coord = ChunkCoord::new(
            read_edit_i32(bytes, 16)?,
            read_edit_i32(bytes, 20)?,
            read_edit_i32(bytes, 24)?,
        );
        if encoded_coord != coord {
            return Err(EditChunkCodecError::CoordinateMismatch);
        }
        let expected_len = EDIT_CHUNK_HEADER_BYTES
            .checked_add(
                count
                    .checked_mul(EDIT_CHUNK_ENTRY_BYTES)
                    .ok_or(EditChunkCodecError::InvalidHeader("entry length overflow"))?,
            )
            .ok_or(EditChunkCodecError::InvalidHeader(
                "payload length overflow",
            ))?;
        if bytes.len() < expected_len {
            return Err(EditChunkCodecError::Truncated);
        }
        if bytes.len() != expected_len {
            return Err(EditChunkCodecError::InvalidHeader("trailing bytes"));
        }
        let entries = &bytes[EDIT_CHUNK_HEADER_BYTES..];
        if edit_chunk_hash(coord, entries).as_bytes() != &bytes[28..60] {
            return Err(EditChunkCodecError::CorruptHash);
        }
        if self.chunks.contains_key(&chunk_key(coord)) {
            return Err(EditChunkCodecError::InvalidHeader(
                "duplicate chunk during hydration",
            ));
        }
        let mut decoded = Vec::with_capacity(count);
        let mut previous = None;
        for entry in entries.chunks_exact(EDIT_CHUNK_ENTRY_BYTES) {
            let index = u16::from_le_bytes([entry[0], entry[1]]);
            if usize::from(index) >= crate::CHUNK_VOLUME
                || previous.is_some_and(|previous| previous >= index)
            {
                return Err(EditChunkCodecError::InvalidHeader(
                    "local indices are not strictly ordered",
                ));
            }
            let material_id = u16::from_le_bytes([entry[2], entry[3]]);
            let material = Material::from_id(material_id)
                .ok_or(EditChunkCodecError::UnknownMaterial(material_id))?;
            decoded.push((index, material));
            previous = Some(index);
        }
        self.override_count += decoded.len();
        self.chunks
            .insert(chunk_key(coord), EditedChunk { overrides: decoded });
        Ok(())
    }

    /// Returns edited X/Z columns inside half-open bounds in deterministic order. Source-neutral
    /// far-LOD composers use this sparse index to sample only columns that could need promotion to
    /// a coarse cell instead of scanning the full tile at canonical resolution.
    pub(crate) fn edited_column_coordinates_in(&self, bounds: [[i32; 2]; 2]) -> Vec<(i32, i32)> {
        self.edited_columns_in(bounds).into_iter().collect()
    }

    /// Conservative far-LOD additions within half-open X/Z bounds. Excavations remain represented
    /// by the regular center sample, while an off-center player-built silhouette cannot disappear
    /// merely because it missed the coarse sample point.
    pub(crate) fn collidable_edited_surface_columns_in(
        &self,
        generator: Generator,
        bounds: [[i32; 2]; 2],
    ) -> Vec<(i32, i32, i32, Material)> {
        let mut columns = Vec::new();
        for (x, z) in self.edited_columns_in(bounds) {
            let (height, material) = self.surface_sample(generator, x, z);
            if material.is_collidable()
                && self.override_at(VoxelCoord::new(x, height, z)) == Some(material)
            {
                columns.push((x, z, height, material));
            }
        }
        columns
    }

    pub fn apply_to_chunk(&self, chunk: &mut Chunk) {
        let coord = chunk.coord();
        let Some(overrides) = self.chunks.get(&chunk_key(coord)) else {
            return;
        };
        for &(index, material) in &overrides.overrides {
            let [x, y, z] = local_coord(index);
            chunk.set(x, y, z, material);
        }
    }

    /// Captures pristine values for the sparse overrides that touch a generated chunk. Clients can
    /// retain these few values for edit comparison and exact reversion without keeping a second
    /// full pristine chunk cache or issuing a point source request later.
    pub fn source_values_for_overrides(&self, chunk: &Chunk) -> Vec<(VoxelCoord, Material)> {
        let coord = chunk.coord();
        let Some(overrides) = self.chunks.get(&chunk_key(coord)) else {
            return Vec::new();
        };
        let origin = coord.world_origin();
        overrides
            .overrides
            .iter()
            .map(|&(index, _)| {
                let local = local_coord(index);
                (
                    VoxelCoord::new(
                        origin[0] + local[0] as i32,
                        origin[1] + local[1] as i32,
                        origin[2] + local[2] as i32,
                    ),
                    chunk.get(local[0], local[1], local[2]),
                )
            })
            .collect()
    }

    /// Returns whether a disposable skyline proxy still represents pristine canonical feature
    /// voxels. The query walks only chunk-index buckets touched by the small analytic feature, so
    /// unrelated large edit journals do not affect surface-tile generation cost.
    pub fn skyline_feature_is_pristine(
        &self,
        generator: Generator,
        feature: SkylineFeature,
    ) -> bool {
        self.skyline_feature_is_pristine_with(feature, |coord| {
            generator.sample(coord.x, coord.y, coord.z)
        })
    }

    /// Source-agnostic pristine check over the bounded feature coordinates touched by edits.
    pub fn skyline_feature_is_pristine_with(
        &self,
        feature: SkylineFeature,
        mut generated_at: impl FnMut(VoxelCoord) -> Material,
    ) -> bool {
        if self.is_empty() {
            return true;
        }
        let [min, max] = feature.bounds();
        let chunk_min = VoxelCoord::new(min[0], min[1], min[2]).chunk();
        let chunk_max = VoxelCoord::new(max[0] - 1, max[1] - 1, max[2] - 1).chunk();
        for chunk_z in chunk_min.z..=chunk_max.z {
            for chunk_y in chunk_min.y..=chunk_max.y {
                for chunk_x in chunk_min.x..=chunk_max.x {
                    let Some(overrides) = self.chunks.get(&(chunk_x, chunk_z, chunk_y)) else {
                        continue;
                    };
                    let origin = ChunkCoord::new(chunk_x, chunk_y, chunk_z).world_origin();
                    for &(index, override_material) in &overrides.overrides {
                        let [local_x, local_y, local_z] = local_coord(index);
                        let coord = VoxelCoord::new(
                            origin[0] + local_x as i32,
                            origin[1] + local_y as i32,
                            origin[2] + local_z as i32,
                        );
                        if coord.x < min[0]
                            || coord.x >= max[0]
                            || coord.y < min[1]
                            || coord.y >= max[1]
                            || coord.z < min[2]
                            || coord.z >= max[2]
                        {
                            continue;
                        }
                        let Some(feature_material) = feature.material_at(coord) else {
                            continue;
                        };
                        if generated_at(coord) == feature_material
                            && override_material != feature_material
                        {
                            return false;
                        }
                    }
                }
            }
        }
        true
    }

    /// Resolves the visible material of one edited column. Far surface summaries use this to remain
    /// derived from generator + edits rather than silently becoming a second world authority.
    pub fn surface_sample(&self, generator: Generator, x: i32, z: i32) -> (i32, Material) {
        let generated = generator.surface_sample(x, z);
        self.surface_sample_with(
            x,
            z,
            (generated.height, generated.material),
            i32::MIN,
            |coord| generator.sample(coord.x, coord.y, coord.z),
        )
    }

    /// Resolves one edited surface column from prepared source data and a bounded column sampler.
    /// `generated_min_y` is the inclusive floor of the prepared product, preventing an accidental
    /// scan outside the caller's authoritative data.
    pub fn surface_sample_with(
        &self,
        x: i32,
        z: i32,
        generated_surface: (i32, Material),
        generated_min_y: i32,
        mut generated_at: impl FnMut(VoxelCoord) -> Material,
    ) -> (i32, Material) {
        let chunk_x = x.div_euclid(CHUNK_EDGE as i32);
        let chunk_z = z.div_euclid(CHUNK_EDGE as i32);
        let local_x = x.rem_euclid(CHUNK_EDGE as i32) as usize;
        let local_z = z.rem_euclid(CHUNK_EDGE as i32) as usize;
        let mut saw_override = false;
        let mut highest_override = None;
        for (&(_, _, chunk_y), chunk) in self
            .chunks
            .range((chunk_x, chunk_z, i32::MIN)..=(chunk_x, chunk_z, i32::MAX))
        {
            let origin_y = chunk_y * CHUNK_EDGE as i32;
            for &(index, material) in chunk.column(local_x, local_z) {
                let entry_y = local_coord(index)[1];
                saw_override = true;
                if material.is_collidable() {
                    let y = origin_y + entry_y as i32;
                    highest_override =
                        Some(highest_override.map_or(y, |highest: i32| highest.max(y)));
                }
            }
        }
        if !saw_override {
            return generated_surface;
        }
        let mut y =
            highest_override.map_or(generated_surface.0, |value| value.max(generated_surface.0));
        loop {
            let coord = VoxelCoord::new(x, y, z);
            let material = self
                .override_at(coord)
                .unwrap_or_else(|| generated_at(coord));
            if material.is_collidable() || y <= generated_min_y {
                return (y, material);
            }
            y -= 1;
        }
    }

    fn replace_override(&mut self, coord: VoxelCoord, material: Option<Material>) {
        let previous = self.override_at(coord);
        if previous == material {
            return;
        }
        let chunk = coord.chunk();
        let key = chunk_key(chunk);
        let local = local_index(coord.local());
        if let Some(material) = material {
            self.chunks
                .entry(key)
                .or_default()
                .replace(local, Some(material));
            if previous.is_none() {
                self.override_count += 1;
            }
            return;
        }

        let remove_chunk = self.chunks.get_mut(&key).is_some_and(|chunk| {
            chunk.replace(local, None);
            chunk.is_empty()
        });
        if remove_chunk {
            self.chunks.remove(&key);
        }
        self.override_count -= 1;
    }

    /// Chunks whose meshes can change after this voxel changes. The mesher samples a full one-voxel
    /// shell for face visibility and ambient occlusion, so edge and corner edits invalidate the
    /// Cartesian product of all touching chunk owners, not only face neighbors.
    pub fn affected_chunks(coord: VoxelCoord) -> Vec<ChunkCoord> {
        let edge = CHUNK_EDGE as i32;
        let local = [
            coord.x.rem_euclid(edge),
            coord.y.rem_euclid(edge),
            coord.z.rem_euclid(edge),
        ];
        let base = coord.chunk();
        let axis_offsets = local.map(|value| {
            if value == 0 {
                [0, -1]
            } else if value == edge - 1 {
                [0, 1]
            } else {
                [0, 0]
            }
        });
        let mut chunks = Vec::with_capacity(8);
        for x in axis_offsets[0] {
            for y in axis_offsets[1] {
                for z in axis_offsets[2] {
                    let Some(x) = base.x.checked_add(x) else {
                        continue;
                    };
                    let Some(y) = base.y.checked_add(y) else {
                        continue;
                    };
                    let Some(z) = base.z.checked_add(z) else {
                        continue;
                    };
                    let neighbor = ChunkCoord::new(x, y, z);
                    if neighbor.is_world_representable() && !chunks.contains(&neighbor) {
                        chunks.push(neighbor);
                    }
                }
            }
        }
        chunks
    }
}

const fn chunk_key(coord: ChunkCoord) -> (i32, i32, i32) {
    (coord.x, coord.z, coord.y)
}

fn local_index([x, y, z]: [usize; 3]) -> u16 {
    debug_assert!(x < CHUNK_EDGE && y < CHUNK_EDGE && z < CHUNK_EDGE);
    let index = y + z * CHUNK_EDGE + x * CHUNK_EDGE * CHUNK_EDGE;
    debug_assert!(u16::try_from(index).is_ok());
    index as u16
}

fn local_coord(index: u16) -> [usize; 3] {
    let index = usize::from(index);
    let x = index / (CHUNK_EDGE * CHUNK_EDGE);
    let within_column_plane = index % (CHUNK_EDGE * CHUNK_EDGE);
    let z = within_column_plane / CHUNK_EDGE;
    let y = within_column_plane % CHUNK_EDGE;
    [x, y, z]
}

fn merge_chunk_overrides(
    existing: Vec<(u16, Material)>,
    changes: &[(u16, Option<Material>)],
) -> Vec<(u16, Material)> {
    let mut merged = Vec::with_capacity(existing.len() + changes.len());
    let mut old = existing.into_iter().peekable();
    let mut new = changes.iter().copied().peekable();
    while old.peek().is_some() || new.peek().is_some() {
        match (old.peek().copied(), new.peek().copied()) {
            (Some((old_index, old_material)), Some((new_index, new_material))) => {
                if old_index < new_index {
                    merged.push((old_index, old_material));
                    old.next();
                } else if new_index < old_index {
                    if let Some(material) = new_material {
                        merged.push((new_index, material));
                    }
                    new.next();
                } else {
                    if let Some(material) = new_material {
                        merged.push((new_index, material));
                    }
                    old.next();
                    new.next();
                }
            }
            (Some(value), None) => {
                merged.push(value);
                old.next();
            }
            (None, Some((index, material))) => {
                if let Some(material) = material {
                    merged.push((index, material));
                }
                new.next();
            }
            (None, None) => break,
        }
    }
    merged
}

fn edit_chunk_hash(coord: ChunkCoord, entries: &[u8]) -> blake3::Hash {
    let mut hasher = blake3::Hasher::new();
    hasher.update(EDIT_CHUNK_HASH_DOMAIN);
    hasher.update(&coord.x.to_le_bytes());
    hasher.update(&coord.y.to_le_bytes());
    hasher.update(&coord.z.to_le_bytes());
    hasher.update(entries);
    hasher.finalize()
}

fn read_edit_u16(bytes: &[u8], offset: usize) -> Result<u16, EditChunkCodecError> {
    let value = bytes
        .get(offset..offset + 2)
        .ok_or(EditChunkCodecError::Truncated)?;
    Ok(u16::from_le_bytes([value[0], value[1]]))
}

fn read_edit_u32(bytes: &[u8], offset: usize) -> Result<u32, EditChunkCodecError> {
    let value = bytes
        .get(offset..offset + 4)
        .ok_or(EditChunkCodecError::Truncated)?;
    Ok(u32::from_le_bytes([value[0], value[1], value[2], value[3]]))
}

fn read_edit_i32(bytes: &[u8], offset: usize) -> Result<i32, EditChunkCodecError> {
    let value = bytes
        .get(offset..offset + 4)
        .ok_or(EditChunkCodecError::Truncated)?;
    Ok(i32::from_le_bytes([value[0], value[1], value[2], value[3]]))
}

/// Applies an authoritative mutation batch to already resident canonical chunks and mesh halos.
///
/// A commit already contains final voxel values, so patching the bounded resident cache is both
/// faster and more coherent than fetching the same chunks again. The owning chunk feeds physics and
/// raycasts; every touched neighbor halo feeds remeshing across chunk faces, edges, and corners.
pub fn apply_resident_mutations(
    chunks: &mut BTreeMap<(i32, i32, i32), Chunk>,
    halos: &mut BTreeMap<(i32, i32, i32), MeshingHalo>,
    mutations: &[crate::protocol::VoxelMutation],
) {
    for mutation in mutations {
        let owner = mutation.coord.chunk();
        if let Some(chunk) = chunks.get_mut(&(owner.x, owner.y, owner.z)) {
            let [x, y, z] = mutation.coord.local();
            chunk.set(x, y, z, mutation.material);
        }
        for coord in EditMap::affected_chunks(mutation.coord) {
            if let Some(halo) = halos.get_mut(&(coord.x, coord.y, coord.z)) {
                halo.set_world(
                    mutation.coord.x,
                    mutation.coord.y,
                    mutation.coord.z,
                    mutation.material,
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durable_edit_chunk_round_trips_compactly_and_binds_its_coordinate() {
        let coord = ChunkCoord::new(-3, 2, 7);
        let origin = coord.world_origin();
        let overrides = [
            (
                VoxelCoord::new(origin[0], origin[1], origin[2]),
                Material::Air,
            ),
            (
                VoxelCoord::new(origin[0] + 9, origin[1] + 4, origin[2] + 2),
                Material::Clay,
            ),
            (
                VoxelCoord::new(origin[0] + 31, origin[1] + 31, origin[2] + 31),
                Material::GlowCrystal,
            ),
        ];
        let mut edits = EditMap::default();
        for (voxel, material) in overrides {
            edits.insert_override(voxel, material);
        }

        let encoded = edits
            .encode_chunk_overrides(coord)
            .expect("edited chunk encoding");
        assert_eq!(
            encoded.len(),
            EDIT_CHUNK_HEADER_BYTES + overrides.len() * EDIT_CHUNK_ENTRY_BYTES
        );
        let mut decoded = EditMap::default();
        decoded
            .decode_chunk_overrides(coord, &encoded)
            .expect("valid edit chunk");
        for (voxel, material) in overrides {
            assert_eq!(decoded.override_at(voxel), Some(material));
        }
        assert_eq!(
            EditMap::default()
                .decode_chunk_overrides(ChunkCoord::new(-3, 2, 8), &encoded)
                .expect_err("coordinate substitution must fail"),
            EditChunkCodecError::CoordinateMismatch
        );

        let mut corrupt = encoded;
        *corrupt.last_mut().expect("encoded entry") ^= 1;
        assert_eq!(
            EditMap::default()
                .decode_chunk_overrides(coord, &corrupt)
                .expect_err("corruption must fail"),
            EditChunkCodecError::CorruptHash
        );
    }

    #[test]
    fn column_snapshot_keeps_all_heights_but_only_requested_horizontal_cells() {
        let mut edits = EditMap::default();
        let requested_low = VoxelCoord::new(-33, -500, 31);
        let requested_high = VoxelCoord::new(-33, 900, 31);
        let adjacent = VoxelCoord::new(-32, 900, 31);
        edits.replace_durable_overrides(&[
            (requested_low, Some(Material::Stone)),
            (requested_high, Some(Material::Wood)),
            (adjacent, Some(Material::Basalt)),
        ]);

        let snapshot = edits.snapshot_for_voxel_columns(-33, -33, 31, 31);
        assert_eq!(snapshot.len(), 2);
        assert_eq!(snapshot.override_at(requested_low), Some(Material::Stone));
        assert_eq!(snapshot.override_at(requested_high), Some(Material::Wood));
        assert_eq!(snapshot.override_at(adjacent), None);
    }

    #[test]
    fn reverting_to_generated_value_removes_override() {
        let generator = Generator::new(7);
        let coord = VoxelCoord::new(4, 30, -2);
        let generated = generator.sample(coord.x, coord.y, coord.z);
        let replacement = if generated == Material::Air {
            Material::Stone
        } else {
            Material::Air
        };
        let mut edits = EditMap::default();
        edits.set(generator, coord, replacement);
        assert_eq!(edits.sample(generator, coord), replacement);
        assert_eq!(edits.len(), 1);
        edits.set(generator, coord, generated);
        assert!(edits.is_empty());
    }

    #[test]
    fn prepared_source_values_drive_sparse_comparison_and_resolution() {
        let coord = VoxelCoord::new(-4, 30, 9);
        let mut edits = EditMap::default();
        edits.set_against_generated(coord, Material::Stone, Material::Air);
        assert_eq!(
            edits.resolve_generated(coord, Material::Air),
            Material::Stone
        );
        edits.set_against_generated(coord, Material::Air, Material::Air);
        assert!(edits.is_empty());
    }

    #[test]
    fn pristine_chunk_values_are_captured_before_sparse_overrides_are_applied() {
        let generator = Generator::new(0x5eed_cafe);
        let coord = VoxelCoord::new(-33, 7, 65);
        let chunk = generator.generate_chunk(coord.chunk());
        let [x, y, z] = coord.local();
        let pristine = chunk.get(x, y, z);
        let replacement = if pristine == Material::Air {
            Material::Stone
        } else {
            Material::Air
        };
        let mut edits = EditMap::default();
        edits.insert_override(coord, replacement);

        assert_eq!(
            edits.source_values_for_overrides(&chunk),
            vec![(coord, pristine)]
        );
        let mut edited = chunk;
        edits.apply_to_chunk(&mut edited);
        assert_eq!(edited.get(x, y, z), replacement);
    }

    #[test]
    fn prepared_surface_sampling_is_lazy_and_stops_at_the_supplied_block_floor() {
        let x = 4;
        let z = -7;
        let removed_surface = VoxelCoord::new(x, 10, z);
        let mut edits = EditMap::default();
        edits.replace_durable_override(removed_surface, Some(Material::Air));
        let mut sampled = Vec::new();
        let resolved = edits.surface_sample_with(x, z, (10, Material::Stone), 9, |coord| {
            sampled.push(coord);
            assert_ne!(
                coord, removed_surface,
                "override must win before source lookup"
            );
            Material::Stone
        });
        assert_eq!(resolved, (9, Material::Stone));
        assert_eq!(sampled, vec![VoxelCoord::new(x, 9, z)]);
    }

    #[test]
    fn generated_water_can_be_removed_and_restored_sparsely() {
        let generator = Generator::new(0x5eed_cafe);
        let coord = VoxelCoord::new(18_016, crate::SEA_LEVEL_VOXELS, 12_896);
        assert_eq!(generator.sample(coord.x, coord.y, coord.z), Material::Water);
        let mut edits = EditMap::default();
        edits.set(generator, coord, Material::Air);
        assert_eq!(edits.sample(generator, coord), Material::Air);
        assert_eq!(edits.override_at(coord), Some(Material::Air));
        edits.set(generator, coord, Material::Water);
        assert_eq!(edits.sample(generator, coord), Material::Water);
        assert!(edits.is_empty());
    }

    #[test]
    fn corner_edit_invalidates_all_eight_chunks_sampled_for_ambient_occlusion() {
        let chunks = EditMap::affected_chunks(VoxelCoord::new(-1, 64, 31));
        assert_eq!(chunks.len(), 8);
        for x in [-1, 0] {
            for y in [1, 2] {
                for z in [0, 1] {
                    assert!(chunks.contains(&ChunkCoord::new(x, y, z)));
                }
            }
        }
    }

    #[test]
    fn world_boundary_edits_never_invalidate_unrepresentable_chunks() {
        for axis in 0..3 {
            for boundary in [i32::MIN, i32::MAX] {
                let mut voxel = [7, 7, 7];
                voxel[axis] = boundary;
                let chunks =
                    EditMap::affected_chunks(VoxelCoord::new(voxel[0], voxel[1], voxel[2]));

                assert_eq!(chunks.len(), 1);
                assert!(chunks[0].is_world_representable());
                assert_eq!(
                    chunks[0],
                    VoxelCoord::new(voxel[0], voxel[1], voxel[2]).chunk()
                );
            }
        }
    }

    #[test]
    fn surface_snapshots_include_positive_world_boundary_edits() {
        for coord in [
            VoxelCoord::new(i32::MAX, 17, 7),
            VoxelCoord::new(7, 17, i32::MAX),
        ] {
            let mut edits = EditMap::default();
            edits.insert_override(coord, Material::Basalt);
            let tile =
                SurfaceTileCoord::containing(crate::SurfaceLodLevel::Stride256, coord.x, coord.z);

            let snapshot = edits.snapshot_for_surface_tiles(&[tile]);

            assert_eq!(snapshot.override_at(coord), Some(Material::Basalt));
        }
    }

    #[test]
    fn chunk_snapshots_union_only_the_requested_meshing_shells() {
        let mut edits = EditMap::default();
        let first_shell = VoxelCoord::new(CHUNK_EDGE as i32, 7, 7);
        let second_shell = VoxelCoord::new(CHUNK_EDGE as i32 * 2 - 1, 7, 7);
        let second_core = VoxelCoord::new(CHUNK_EDGE as i32 * 2, 7, 7);
        let gap = VoxelCoord::new(CHUNK_EDGE as i32 + CHUNK_EDGE as i32 / 2, 7, 7);
        for coord in [first_shell, second_shell, second_core, gap] {
            edits.insert_override(coord, Material::Basalt);
        }

        let snapshot =
            edits.snapshot_for_chunks(&[ChunkCoord::new(0, 0, 0), ChunkCoord::new(2, 0, 0)]);

        assert_eq!(snapshot.override_at(first_shell), Some(Material::Basalt));
        assert_eq!(snapshot.override_at(second_shell), Some(Material::Basalt));
        assert_eq!(snapshot.override_at(second_core), Some(Material::Basalt));
        assert_eq!(snapshot.override_at(gap), None);
        assert_eq!(snapshot.len(), 3);
    }

    #[test]
    fn edited_column_surface_tracks_additions_and_removals() {
        let generator = Generator::new(11);
        let x = 7;
        let z = -9;
        let generated = generator.surface_height(x, z);
        let mut edits = EditMap::default();
        edits.set(
            generator,
            VoxelCoord::new(x, generated + 5, z),
            Material::Stone,
        );
        assert_eq!(edits.surface_sample(generator, x, z).0, generated + 5);
        edits.set(
            generator,
            VoxelCoord::new(x, generated + 5, z),
            Material::Air,
        );
        edits.set(generator, VoxelCoord::new(x, generated, z), Material::Air);
        assert_eq!(edits.surface_sample(generator, x, z).0, generated - 1);
    }

    #[test]
    fn edited_column_surface_reaches_the_generated_floor() {
        let generator = Generator::new(11);
        let x = 7;
        let z = -9;
        let generated = generator.surface_height(x, z);
        let mut edits = EditMap::default();
        for y in -16..=generated {
            edits.set(generator, VoxelCoord::new(x, y, z), Material::Air);
        }

        assert_eq!(generator.sample(x, -17, z), Material::Basalt);
        assert_eq!(
            edits.surface_sample(generator, x, z),
            (-17, Material::Basalt)
        );
    }

    #[test]
    fn compact_chunk_index_follows_override_replacement() {
        let generator = Generator::new(19);
        let coord = VoxelCoord::new(-33, 65, 31);
        let mut edits = EditMap::default();
        edits.insert_override(coord, Material::Snow);
        edits.insert_override(coord, Material::Basalt);
        assert_eq!(edits.len(), 1);
        assert_eq!(edits.override_at(coord), Some(Material::Basalt));

        let mut chunk = generator.generate_chunk(coord.chunk());
        edits.apply_to_chunk(&mut chunk);
        let [x, y, z] = coord.local();
        assert_eq!(chunk.get(x, y, z), Material::Basalt);
        assert_eq!(edits.surface_sample(generator, coord.x, coord.z).0, coord.y);

        edits.set(
            generator,
            coord,
            generator.sample(coord.x, coord.y, coord.z),
        );
        assert!(edits.is_empty());
        assert!(edits.chunks.is_empty());
    }

    #[test]
    fn committed_remote_override_can_be_inserted_and_removed() {
        let coord = VoxelCoord::new(7, 8, 9);
        let mut edits = EditMap::default();
        edits.replace_durable_override(coord, Some(Material::Clay));
        assert_eq!(edits.override_at(coord), Some(Material::Clay));
        edits.replace_durable_override(coord, None);
        assert_eq!(edits.override_at(coord), None);
        assert!(edits.chunks.is_empty());
    }

    #[test]
    fn redundant_durable_override_keeps_skyline_proxy_pristine() {
        let generator = Generator::new(0x5eed);
        let feature = generator
            .nearest_skyline_feature(0, 0, crate::SkylineFeatureKind::Broadleaf, 128)
            .expect("fixed catalog should contain a nearby broadleaf");
        let coord = VoxelCoord::new(feature.anchor[0], feature.anchor[1] + 1, feature.anchor[2]);
        let generated = generator.sample(coord.x, coord.y, coord.z);
        assert_eq!(feature.material_at(coord), Some(generated));

        let mut edits = EditMap::default();
        edits.insert_override(coord, generated);

        assert!(edits.skyline_feature_is_pristine(generator, feature));
        edits.insert_override(coord, Material::Air);
        assert!(!edits.skyline_feature_is_pristine(generator, feature));
    }

    #[test]
    fn logical_memory_tracks_rows_without_growing_on_replacement() {
        let coord = VoxelCoord::new(7, 8, 9);
        let mut edits = EditMap::default();
        let empty = edits.logical_bytes();
        edits.insert_override(coord, Material::Clay);
        let one_row = edits.logical_bytes();
        edits.insert_override(coord, Material::Basalt);
        assert_eq!(edits.logical_bytes(), one_row);
        assert!(one_row > empty);
        edits.replace_durable_override(coord, None);
        assert_eq!(edits.logical_bytes(), empty);
    }

    #[test]
    fn authoritative_edits_patch_resident_core_and_every_boundary_halo() {
        let target = VoxelCoord::new(31, 31, 31);
        let affected = EditMap::affected_chunks(target);
        assert_eq!(
            affected.len(),
            8,
            "a corner edit must invalidate eight chunks"
        );

        let mut chunks = BTreeMap::new();
        let mut halos = BTreeMap::new();
        for coord in &affected {
            chunks.insert(
                (coord.x, coord.y, coord.z),
                Chunk::filled(*coord, Material::Stone),
            );
            halos.insert(
                (coord.x, coord.y, coord.z),
                MeshingHalo::from_sampler(*coord, |_, _, _| Material::Stone),
            );
        }

        apply_resident_mutations(
            &mut chunks,
            &mut halos,
            &[crate::protocol::VoxelMutation {
                coord: target,
                material: Material::Air,
            }],
        );

        let owner = target.chunk();
        let [x, y, z] = target.local();
        assert_eq!(
            chunks[&(owner.x, owner.y, owner.z)].get(x, y, z),
            Material::Air,
            "physics and raycasts must see the authoritative value immediately"
        );
        for coord in affected {
            let sampled =
                halos[&(coord.x, coord.y, coord.z)].sample_world(target.x, target.y, target.z);
            if coord == owner {
                assert_eq!(sampled, None, "an owner's halo must exclude its core");
            } else {
                assert_eq!(
                    sampled,
                    Some(Material::Air),
                    "every neighboring mesh halo must receive the boundary edit"
                );
            }
        }
    }

    #[test]
    fn rapid_place_dig_and_cross_chunk_batches_leave_one_canonical_final_state() {
        let coords = [
            VoxelCoord::new(-1, 7, 31),
            VoxelCoord::new(0, 7, 31),
            VoxelCoord::new(31, 7, 31),
            VoxelCoord::new(32, 7, 32),
        ];
        let resident_coords = coords
            .iter()
            .flat_map(|coord| EditMap::affected_chunks(*coord))
            .collect::<std::collections::BTreeSet<_>>();
        let mut chunks = resident_coords
            .iter()
            .map(|coord| {
                (
                    (coord.x, coord.y, coord.z),
                    Chunk::filled(*coord, Material::Air),
                )
            })
            .collect::<BTreeMap<_, _>>();
        let mut halos = resident_coords
            .iter()
            .map(|coord| {
                (
                    (coord.x, coord.y, coord.z),
                    MeshingHalo::from_sampler(*coord, |_, _, _| Material::Air),
                )
            })
            .collect::<BTreeMap<_, _>>();

        let placed = coords
            .iter()
            .copied()
            .map(|coord| crate::protocol::VoxelMutation {
                coord,
                material: Material::Basalt,
            })
            .collect::<Vec<_>>();
        apply_resident_mutations(&mut chunks, &mut halos, &placed);
        apply_resident_mutations(
            &mut chunks,
            &mut halos,
            &[
                crate::protocol::VoxelMutation {
                    coord: coords[0],
                    material: Material::Air,
                },
                crate::protocol::VoxelMutation {
                    coord: coords[2],
                    material: Material::GlowCrystal,
                },
            ],
        );

        let expected = [
            Material::Air,
            Material::Basalt,
            Material::GlowCrystal,
            Material::Basalt,
        ];
        for (coord, expected) in coords.into_iter().zip(expected) {
            let owner = coord.chunk();
            let [x, y, z] = coord.local();
            assert_eq!(chunks[&(owner.x, owner.y, owner.z)].get(x, y, z), expected);
            for affected in EditMap::affected_chunks(coord) {
                if affected != owner {
                    assert_eq!(
                        halos[&(affected.x, affected.y, affected.z)]
                            .sample_world(coord.x, coord.y, coord.z),
                        Some(expected)
                    );
                }
            }
        }
    }
}
