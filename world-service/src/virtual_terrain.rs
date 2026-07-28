//! Server-side producer and bounded content cache for virtual microvoxel terrain.
//!
//! A cache entry owns one complete fixed hierarchy region. Directories and pages are always
//! derived from the same atomic edit snapshot, while page payloads are retained in their encoded
//! content-addressed form so the configured byte bound describes actual cache memory rather than
//! an optimistic compressed-size estimate.

use crate::edits::{EditAuthority, TerrainEditSnapshot};
use crate::generation_limiter::PriorityGenerationLimiter;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use tokio::sync::{Mutex as AsyncMutex, Semaphore};
use voxels_world::{
    Material, TERRAIN_PAGE_EDGE_SAMPLES, TERRAIN_PAGE_TARGET_COMPRESSED_BYTES,
    TERRAIN_REGION_ROOT_LEVEL, TerrainHierarchyDirectoryV1, TerrainPageKey,
    TerrainPageTransferIdentity, TerrainPageV1, TerrainRegionBuildV1, TerrainSimplificationBudget,
    VoxelBlockRequest, VoxelCoord, WorldProduct, WorldProductBatch, WorldProductPriority,
    WorldProductRequest, WorldSourceEngine, WorldSourceIdentityHash, build_terrain_region,
    decode_terrain_page, encode_terrain_directory, encode_terrain_page,
};

const REGION_SAMPLE_EDGE: u32 = (TERRAIN_PAGE_EDGE_SAMPLES << TERRAIN_REGION_ROOT_LEVEL) + 2;
const REGION_SAMPLE_YZ_SEGMENT_EDGE: u32 = 65;
const REGION_COLUMN_FEATURE_MARGIN_VOXELS: i32 = 18;
const MAX_REGION_ROOTS_PER_COLUMN: usize = 64;
const REGION_BUILD_ATTEMPTS: usize = 3;
const REGION_TARGET_TRIANGLES: u32 = 8_192;
const REGION_MAX_ERROR_MILLIVOXELS: u32 = 4_000;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum VirtualTerrainError {
    InvalidRoot,
    Source(String),
    Build(String),
    Codec(String),
    ChangedDuringBuild,
    TaskFailed,
}

impl fmt::Display for VirtualTerrainError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRoot => formatter.write_str("invalid virtual terrain region root"),
            Self::Source(reason) => write!(formatter, "virtual terrain source failed: {reason}"),
            Self::Build(reason) => write!(formatter, "virtual terrain build failed: {reason}"),
            Self::Codec(reason) => write!(formatter, "virtual terrain codec failed: {reason}"),
            Self::ChangedDuringBuild => {
                formatter.write_str("virtual terrain region changed repeatedly during generation")
            }
            Self::TaskFailed => formatter.write_str("virtual terrain generation task failed"),
        }
    }
}

impl std::error::Error for VirtualTerrainError {}

#[derive(Clone)]
pub(crate) struct PreparedTerrainRegion {
    pub(crate) root: TerrainPageKey,
    pub(crate) revision: u64,
    pub(crate) directory: TerrainHierarchyDirectoryV1,
    pages: BTreeMap<TerrainPageKey, Arc<[u8]>>,
    retained_bytes: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PreparedTerrainRegionColumn {
    pub(crate) column: [i32; 2],
    pub(crate) revision: u64,
    pub(crate) roots: Vec<TerrainPageKey>,
}

impl PreparedTerrainRegion {
    pub(crate) fn page(
        &self,
        identity: TerrainPageTransferIdentity,
    ) -> Result<Option<TerrainPageV1>, VirtualTerrainError> {
        let Some(node) = self.directory.node(identity.key) else {
            return Ok(None);
        };
        if node.revision != identity.revision
            || node.content_fingerprint != identity.content_fingerprint
        {
            return Ok(None);
        }
        let Some(encoded) = self.pages.get(&identity.key) else {
            return Ok(None);
        };
        let page = decode_terrain_page(encoded, self.directory.source_identity_hash)
            .map_err(|error| VirtualTerrainError::Codec(error.to_string()))?;
        if !identity.matches(&page) {
            return Err(VirtualTerrainError::Codec(
                "cached page identity does not match its directory".to_owned(),
            ));
        }
        Ok(Some(page))
    }

    pub(crate) fn root_page(&self) -> Result<TerrainPageV1, VirtualTerrainError> {
        let node = self
            .directory
            .node(self.root)
            .ok_or_else(|| VirtualTerrainError::Build("region root is absent".to_owned()))?;
        self.page(TerrainPageTransferIdentity {
            key: node.key,
            revision: node.revision,
            content_fingerprint: node.content_fingerprint,
        })?
        .ok_or_else(|| VirtualTerrainError::Build("region root payload is absent".to_owned()))
    }
}

struct CachedRegion {
    region: Arc<PreparedTerrainRegion>,
    last_access: u64,
}

struct RegionCache {
    max_bytes: usize,
    retained_bytes: usize,
    entries: BTreeMap<TerrainPageKey, CachedRegion>,
    lru: BTreeSet<(u64, TerrainPageKey)>,
    next_access: u64,
}

impl RegionCache {
    fn new(max_bytes: usize) -> Self {
        Self {
            max_bytes,
            retained_bytes: 0,
            entries: BTreeMap::new(),
            lru: BTreeSet::new(),
            next_access: 1,
        }
    }

    fn get(&mut self, root: TerrainPageKey, revision: u64) -> Option<Arc<PreparedTerrainRegion>> {
        let access = self.record_access();
        let entry = self.entries.get_mut(&root)?;
        if entry.region.revision != revision {
            return None;
        }
        self.lru.remove(&(entry.last_access, root));
        entry.last_access = access;
        self.lru.insert((access, root));
        Some(Arc::clone(&entry.region))
    }

    fn insert(&mut self, region: Arc<PreparedTerrainRegion>) {
        let bytes = region.retained_bytes;
        if self.max_bytes == 0 || bytes > self.max_bytes {
            return;
        }
        if let Some(replaced) = self.entries.remove(&region.root) {
            self.retained_bytes = self
                .retained_bytes
                .saturating_sub(replaced.region.retained_bytes);
            self.lru.remove(&(replaced.last_access, region.root));
        }
        while self.retained_bytes.saturating_add(bytes) > self.max_bytes {
            let Some((_, oldest)) = self.lru.pop_first() else {
                break;
            };
            if let Some(evicted) = self.entries.remove(&oldest) {
                self.retained_bytes = self
                    .retained_bytes
                    .saturating_sub(evicted.region.retained_bytes);
            }
        }
        let access = self.record_access();
        self.retained_bytes = self.retained_bytes.saturating_add(bytes);
        self.lru.insert((access, region.root));
        self.entries.insert(
            region.root,
            CachedRegion {
                region,
                last_access: access,
            },
        );
    }

    fn record_access(&mut self) -> u64 {
        let access = self.next_access;
        self.next_access = self.next_access.saturating_add(1);
        access
    }
}

pub(crate) struct VirtualTerrainAuthority {
    source: Arc<dyn WorldSourceEngine>,
    edits: Arc<EditAuthority>,
    generation_limiter: Arc<PriorityGenerationLimiter>,
    region_build_limiter: Arc<Semaphore>,
    cache: Mutex<RegionCache>,
    flights: Mutex<HashMap<TerrainPageKey, Weak<AsyncMutex<()>>>>,
}

impl VirtualTerrainAuthority {
    pub(crate) fn new(
        source: Arc<dyn WorldSourceEngine>,
        edits: Arc<EditAuthority>,
        generation_limiter: Arc<PriorityGenerationLimiter>,
        region_build_workers: usize,
        cache_bytes: usize,
    ) -> Arc<Self> {
        Arc::new(Self {
            source,
            edits,
            generation_limiter,
            region_build_limiter: Arc::new(Semaphore::new(region_build_workers.max(1))),
            cache: Mutex::new(RegionCache::new(cache_bytes)),
            flights: Mutex::new(HashMap::new()),
        })
    }

    pub(crate) fn source_identity_hash(&self) -> WorldSourceIdentityHash {
        self.source.identity().identity_hash()
    }

    pub(crate) fn current_revision(&self, root: TerrainPageKey) -> Option<u64> {
        self.edits.terrain_region_revision(root)
    }

    pub(crate) async fn ensure_region(
        self: &Arc<Self>,
        root: TerrainPageKey,
        priority: WorldProductPriority,
    ) -> Result<Arc<PreparedTerrainRegion>, VirtualTerrainError> {
        let revision = self
            .current_revision(root)
            .ok_or(VirtualTerrainError::InvalidRoot)?;
        if let Some(region) = self.lock_cache().get(root, revision) {
            return Ok(region);
        }

        let flight = self.flight_lock(root);
        let _flight_guard = flight.lock().await;
        let revision = self
            .current_revision(root)
            .ok_or(VirtualTerrainError::InvalidRoot)?;
        if let Some(region) = self.lock_cache().get(root, revision) {
            self.finish_flight(root, &flight);
            return Ok(region);
        }

        let _generation_permit = self.generation_limiter.acquire(priority).await;
        let _region_permit = Arc::clone(&self.region_build_limiter)
            .acquire_owned()
            .await
            .map_err(|_| VirtualTerrainError::TaskFailed)?;
        let authority = Arc::clone(self);
        let generated =
            tokio::task::spawn_blocking(move || authority.build_current_region(root, priority))
                .await
                .map_err(|_| VirtualTerrainError::TaskFailed)
                .and_then(|result| result);
        if let Ok(region) = &generated {
            self.lock_cache().insert(Arc::clone(region));
        }
        self.finish_flight(root, &flight);
        generated
    }

    pub(crate) async fn discover_region_column(
        self: &Arc<Self>,
        column: [i32; 2],
        priority: WorldProductPriority,
    ) -> Result<PreparedTerrainRegionColumn, VirtualTerrainError> {
        let _generation_permit = self.generation_limiter.acquire(priority).await;
        let authority = Arc::clone(self);
        tokio::task::spawn_blocking(move || authority.build_region_column(column, priority))
            .await
            .map_err(|_| VirtualTerrainError::TaskFailed)?
    }

    fn build_current_region(
        &self,
        root: TerrainPageKey,
        priority: WorldProductPriority,
    ) -> Result<Arc<PreparedTerrainRegion>, VirtualTerrainError> {
        for _ in 0..REGION_BUILD_ATTEMPTS {
            let snapshot = self
                .edits
                .snapshot_terrain_region(root)
                .ok_or(VirtualTerrainError::InvalidRoot)?;
            let built = build_region_from_snapshot(
                self.source.as_ref(),
                root,
                snapshot.clone(),
                self.source_identity_hash(),
                priority,
            )?;
            let current_revision = self
                .edits
                .terrain_region_revision(root)
                .ok_or(VirtualTerrainError::InvalidRoot)?;
            if current_revision == snapshot.revision {
                return prepare_region(built);
            }
        }
        Err(VirtualTerrainError::ChangedDuringBuild)
    }

    fn build_region_column(
        &self,
        column: [i32; 2],
        priority: WorldProductPriority,
    ) -> Result<PreparedTerrainRegionColumn, VirtualTerrainError> {
        let horizontal_root = TerrainPageKey {
            level: TERRAIN_REGION_ROOT_LEVEL,
            coord: [column[0], 0, column[1]],
        };
        let bounds = horizontal_root
            .bounds()
            .ok_or(VirtualTerrainError::InvalidRoot)?;
        let surface_bounds = [
            [
                bounds
                    .min
                    .x
                    .checked_sub(1)
                    .ok_or(VirtualTerrainError::InvalidRoot)?,
                bounds
                    .min
                    .z
                    .checked_sub(1)
                    .ok_or(VirtualTerrainError::InvalidRoot)?,
            ],
            [
                bounds
                    .max
                    .x
                    .checked_add(1)
                    .ok_or(VirtualTerrainError::InvalidRoot)?,
                bounds
                    .max
                    .z
                    .checked_add(1)
                    .ok_or(VirtualTerrainError::InvalidRoot)?,
            ],
        ];
        let [mut minimum_y, mut maximum_y] = self
            .source
            .conservative_surface_height_bounds(priority, surface_bounds)
            .map_err(|error| VirtualTerrainError::Source(error.to_string()))?;

        let feature_minimum = [
            bounds
                .min
                .x
                .checked_sub(REGION_COLUMN_FEATURE_MARGIN_VOXELS)
                .ok_or(VirtualTerrainError::InvalidRoot)?,
            bounds
                .min
                .z
                .checked_sub(REGION_COLUMN_FEATURE_MARGIN_VOXELS)
                .ok_or(VirtualTerrainError::InvalidRoot)?,
        ];
        let feature_maximum = [
            bounds
                .max
                .x
                .checked_add(REGION_COLUMN_FEATURE_MARGIN_VOXELS)
                .ok_or(VirtualTerrainError::InvalidRoot)?,
            bounds
                .max
                .z
                .checked_add(REGION_COLUMN_FEATURE_MARGIN_VOXELS)
                .ok_or(VirtualTerrainError::InvalidRoot)?,
        ];
        for feature in self
            .source
            .skyline_features_anchored_in([feature_minimum, feature_maximum])
        {
            let [minimum, maximum] = feature.bounds();
            if minimum[0] < bounds.max.x
                && maximum[0] > bounds.min.x
                && minimum[2] < bounds.max.z
                && maximum[2] > bounds.min.z
            {
                minimum_y = minimum_y.min(minimum[1]);
                maximum_y = maximum_y.max(maximum[1].saturating_sub(1));
            }
        }

        let (revision, edited_chunks) = self
            .edits
            .terrain_region_column_edits(column)
            .ok_or(VirtualTerrainError::InvalidRoot)?;
        let root_span = i32::try_from(TERRAIN_PAGE_EDGE_SAMPLES)
            .map_err(|_| VirtualTerrainError::InvalidRoot)?
            .checked_shl(u32::from(TERRAIN_REGION_ROOT_LEVEL))
            .ok_or(VirtualTerrainError::InvalidRoot)?;
        let mut root_ys = (minimum_y.div_euclid(root_span)..=maximum_y.div_euclid(root_span))
            .collect::<BTreeSet<_>>();
        for chunk in edited_chunks {
            let leaf = TerrainPageKey {
                level: 0,
                coord: [chunk.x, chunk.y, chunk.z],
            };
            if let Some(root) = leaf.ancestor_at(TERRAIN_REGION_ROOT_LEVEL)
                && [root.coord[0], root.coord[2]] == column
            {
                root_ys.insert(root.coord[1]);
            }
        }
        if root_ys.is_empty() || root_ys.len() > MAX_REGION_ROOTS_PER_COLUMN {
            return Err(VirtualTerrainError::Build(
                "terrain region column root count exceeds its fixed bound".to_owned(),
            ));
        }
        let roots = root_ys
            .into_iter()
            .map(|y| TerrainPageKey {
                level: TERRAIN_REGION_ROOT_LEVEL,
                coord: [column[0], y, column[1]],
            })
            .collect::<Vec<_>>();
        if roots.iter().any(|root| root.bounds().is_none()) {
            return Err(VirtualTerrainError::InvalidRoot);
        }
        Ok(PreparedTerrainRegionColumn {
            column,
            revision,
            roots,
        })
    }

    fn lock_cache(&self) -> MutexGuard<'_, RegionCache> {
        match self.cache.lock() {
            Ok(cache) => cache,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    fn lock_flights(&self) -> MutexGuard<'_, HashMap<TerrainPageKey, Weak<AsyncMutex<()>>>> {
        match self.flights.lock() {
            Ok(flights) => flights,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    fn flight_lock(&self, root: TerrainPageKey) -> Arc<AsyncMutex<()>> {
        let mut flights = self.lock_flights();
        if let Some(flight) = flights.get(&root).and_then(Weak::upgrade) {
            return flight;
        }
        let flight = Arc::new(AsyncMutex::new(()));
        flights.insert(root, Arc::downgrade(&flight));
        flight
    }

    fn finish_flight(&self, root: TerrainPageKey, flight: &Arc<AsyncMutex<()>>) {
        let mut flights = self.lock_flights();
        if flights
            .get(&root)
            .and_then(Weak::upgrade)
            .is_some_and(|current| Arc::ptr_eq(&current, flight))
        {
            flights.remove(&root);
        }
    }
}

fn build_region_from_snapshot(
    source: &dyn WorldSourceEngine,
    root: TerrainPageKey,
    snapshot: TerrainEditSnapshot,
    source_identity_hash: WorldSourceIdentityHash,
    priority: WorldProductPriority,
) -> Result<TerrainRegionBuildV1, VirtualTerrainError> {
    let dense = sample_region(source, root, priority)?;
    let mut sample_missed = false;
    let built = build_terrain_region(
        source_identity_hash,
        root,
        snapshot.revision,
        TerrainSimplificationBudget {
            target_triangles: REGION_TARGET_TRIANGLES,
            max_error_millivoxels: REGION_MAX_ERROR_MILLIVOXELS,
            target_encoded_bytes: TERRAIN_PAGE_TARGET_COMPRESSED_BYTES as u32,
        },
        |coord| {
            let Some(generated) = dense.sample(coord) else {
                sample_missed = true;
                return Material::Air;
            };
            snapshot.edits.resolve_generated(coord, generated)
        },
    )
    .map_err(|error| VirtualTerrainError::Build(error.to_string()))?;
    if sample_missed {
        return Err(VirtualTerrainError::Source(
            "region builder sampled outside its canonical halo".to_owned(),
        ));
    }
    Ok(built)
}

fn prepare_region(
    built: TerrainRegionBuildV1,
) -> Result<Arc<PreparedTerrainRegion>, VirtualTerrainError> {
    let mut retained_bytes = encode_terrain_directory(&built.directory)
        .map_err(|error| VirtualTerrainError::Codec(error.to_string()))?
        .len();
    let mut pages = BTreeMap::new();
    for page in built.pages {
        let key = page.key;
        let encoded = encode_terrain_page(&page)
            .map_err(|error| VirtualTerrainError::Codec(error.to_string()))?;
        retained_bytes = retained_bytes.saturating_add(encoded.len());
        if pages.insert(key, Arc::from(encoded)).is_some() {
            return Err(VirtualTerrainError::Build(
                "terrain region produced duplicate page keys".to_owned(),
            ));
        }
    }
    if pages.len() != built.directory.nodes.len() {
        return Err(VirtualTerrainError::Build(
            "terrain region directory and payload count disagree".to_owned(),
        ));
    }
    let revision = built
        .directory
        .nodes
        .first()
        .map(|node| node.revision)
        .ok_or_else(|| VirtualTerrainError::Build("terrain directory is empty".to_owned()))?;
    if built
        .directory
        .nodes
        .iter()
        .any(|node| node.revision != revision)
    {
        return Err(VirtualTerrainError::Build(
            "terrain region contains mixed revisions".to_owned(),
        ));
    }
    Ok(Arc::new(PreparedTerrainRegion {
        root: built.root,
        revision,
        directory: built.directory,
        pages,
        retained_bytes,
    }))
}

struct DenseRegion {
    min: VoxelCoord,
    shape: [usize; 3],
    materials: Vec<Material>,
}

impl DenseRegion {
    fn sample(&self, coord: VoxelCoord) -> Option<Material> {
        let offset = [
            i64::from(coord.x) - i64::from(self.min.x),
            i64::from(coord.y) - i64::from(self.min.y),
            i64::from(coord.z) - i64::from(self.min.z),
        ];
        if offset
            .iter()
            .zip(self.shape)
            .any(|(&component, length)| component < 0 || component >= length as i64)
        {
            return None;
        }
        let [x, y, z] = offset.map(|component| component as usize);
        self.materials
            .get(x + y * self.shape[0] + z * self.shape[0] * self.shape[1])
            .copied()
    }
}

fn sample_region(
    source: &dyn WorldSourceEngine,
    root: TerrainPageKey,
    priority: WorldProductPriority,
) -> Result<DenseRegion, VirtualTerrainError> {
    if root.level != TERRAIN_REGION_ROOT_LEVEL {
        return Err(VirtualTerrainError::InvalidRoot);
    }
    let bounds = root.bounds().ok_or(VirtualTerrainError::InvalidRoot)?;
    let min = VoxelCoord::new(
        bounds
            .min
            .x
            .checked_sub(1)
            .ok_or(VirtualTerrainError::InvalidRoot)?,
        bounds
            .min
            .y
            .checked_sub(1)
            .ok_or(VirtualTerrainError::InvalidRoot)?,
        bounds
            .min
            .z
            .checked_sub(1)
            .ok_or(VirtualTerrainError::InvalidRoot)?,
    );
    let mut requests = Vec::new();
    let mut z_offset = 0_i32;
    while z_offset < REGION_SAMPLE_EDGE as i32 {
        let depth = (REGION_SAMPLE_EDGE - z_offset as u32).min(REGION_SAMPLE_YZ_SEGMENT_EDGE);
        let mut y_offset = 0_i32;
        while y_offset < REGION_SAMPLE_EDGE as i32 {
            let height = (REGION_SAMPLE_EDGE - y_offset as u32).min(REGION_SAMPLE_YZ_SEGMENT_EDGE);
            requests.push(VoxelBlockRequest {
                min: VoxelCoord::new(
                    min.x,
                    min.y
                        .checked_add(y_offset)
                        .ok_or(VirtualTerrainError::InvalidRoot)?,
                    min.z
                        .checked_add(z_offset)
                        .ok_or(VirtualTerrainError::InvalidRoot)?,
                ),
                sample_shape: [REGION_SAMPLE_EDGE, height, depth],
            });
            y_offset += height as i32;
        }
        z_offset += depth as i32;
    }
    let batch = source
        .generate_batch(WorldProductBatch {
            priority,
            requests: requests
                .iter()
                .copied()
                .map(WorldProductRequest::VoxelBlock)
                .collect(),
        })
        .map_err(|error| VirtualTerrainError::Source(error.to_string()))?;
    if batch.source_identity_hash != source.identity().identity_hash()
        || batch.items.len() != requests.len()
    {
        return Err(VirtualTerrainError::Source(
            "source returned a mismatched region sample batch".to_owned(),
        ));
    }
    let edge = REGION_SAMPLE_EDGE as usize;
    let mut dense = DenseRegion {
        min,
        shape: [edge; 3],
        materials: vec![Material::Air; edge * edge * edge],
    };
    for (expected, item) in requests.into_iter().zip(batch.items) {
        let snapshot = match (item.request, item.result) {
            (WorldProductRequest::VoxelBlock(returned), Ok(WorldProduct::VoxelBlock(snapshot)))
                if returned == expected
                    && snapshot.request == expected
                    && snapshot.source_identity_hash == source.identity().identity_hash() =>
            {
                snapshot
            }
            (_, Err(error)) => return Err(VirtualTerrainError::Source(error.to_string())),
            _ => {
                return Err(VirtualTerrainError::Source(
                    "source returned a mismatched voxel block".to_owned(),
                ));
            }
        };
        for z in 0..expected.sample_shape[2] as usize {
            for y in 0..expected.sample_shape[1] as usize {
                let source_start = y * expected.sample_shape[0] as usize
                    + z * expected.sample_shape[0] as usize * expected.sample_shape[1] as usize;
                let world_x = (i64::from(expected.min.x) - i64::from(min.x)) as usize;
                let world_y = (i64::from(expected.min.y) - i64::from(min.y)) as usize + y;
                let world_z = (i64::from(expected.min.z) - i64::from(min.z)) as usize + z;
                let destination_start = world_x + world_y * edge + world_z * edge * edge;
                let width = expected.sample_shape[0] as usize;
                dense.materials[destination_start..destination_start + width]
                    .copy_from_slice(&snapshot.materials()[source_start..source_start + width]);
            }
        }
    }
    Ok(dense)
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "virtual terrain producer tests use direct fixtures"
)]
mod tests {
    use super::*;
    use voxels_world::ProceduralWorldSource;

    fn root() -> TerrainPageKey {
        TerrainPageKey {
            level: TERRAIN_REGION_ROOT_LEVEL,
            coord: [-1, 0, 1],
        }
    }

    #[test]
    fn dense_region_sampling_covers_the_exact_leaf_halo() {
        let source = ProceduralWorldSource::new(17);
        let sampled =
            sample_region(&source, root(), WorldProductPriority::VisibleSurface).expect("sample");
        let bounds = root().bounds().expect("bounds");
        assert!(
            sampled
                .sample(VoxelCoord::new(
                    bounds.min.x - 1,
                    bounds.min.y - 1,
                    bounds.min.z - 1
                ))
                .is_some()
        );
        assert!(sampled.sample(bounds.max).is_some());
        assert!(
            sampled
                .sample(VoxelCoord::new(
                    bounds.max.x + 1,
                    bounds.max.y,
                    bounds.max.z
                ))
                .is_none()
        );
    }
}
