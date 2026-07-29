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
    FEATURE_MAX_RADIUS_VOXELS, Material, TERRAIN_COVERAGE_ROOT_LEVEL, TERRAIN_PAGE_EDGE_SAMPLES,
    TERRAIN_PAGE_TARGET_COMPRESSED_BYTES, TERRAIN_REGION_ROOT_LEVEL, TerrainErrorBounds,
    TerrainHierarchyDirectoryV1, TerrainPageKey, TerrainPageTransferIdentity, TerrainPageV1,
    TerrainRegionBuildV1, TerrainSimplificationBudget, VoxelBlockRequest, VoxelCoord, WorldProduct,
    WorldProductBatch, WorldProductPriority, WorldProductRequest, WorldSourceEngine,
    WorldSourceIdentityHash, build_exact_surface_terrain_page,
    build_terrain_coverage_root_with_revisions, build_terrain_region, decode_terrain_page,
    encode_terrain_directory, encode_terrain_page,
};

const REGION_SAMPLE_EDGE: u32 = (TERRAIN_PAGE_EDGE_SAMPLES << TERRAIN_REGION_ROOT_LEVEL) + 2;
const REGION_SAMPLE_YZ_SEGMENT_EDGE: u32 = 65;
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
        match (root.level, root.is_surface()) {
            (TERRAIN_REGION_ROOT_LEVEL, false) => self.edits.terrain_region_revision(root),
            (1..=TERRAIN_COVERAGE_ROOT_LEVEL, true) => self.edits.surface_terrain_revision(root),
            _ => None,
        }
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
        let flight_guard = Arc::clone(&flight).lock_owned().await;
        let revision = self
            .current_revision(root)
            .ok_or(VirtualTerrainError::InvalidRoot)?;
        if let Some(region) = self.lock_cache().get(root, revision) {
            self.finish_flight(root, &flight);
            return Ok(region);
        }

        // Claim the narrow terrain-build lane before the broader priority generation permit.
        // Otherwise queued terrain tasks can occupy every process-wide permit while waiting for
        // one or two region lanes, preventing collision-critical work from preempting them.
        let region_permit = Arc::clone(&self.region_build_limiter)
            .acquire_owned()
            .await
            .map_err(|_| VirtualTerrainError::TaskFailed)?;
        let generation_permit = self.generation_limiter.acquire(priority).await;
        let authority = Arc::clone(self);
        tokio::task::spawn_blocking(move || {
            // Once native generation starts, it is not abortable. Own the single-flight lock and
            // cache publication inside this task so a browser request timeout cannot discard the
            // completed region and force every retry to repeat the same expensive build.
            let _flight_guard = flight_guard;
            let _generation_permit = generation_permit;
            let _region_permit = region_permit;
            let generated = authority.build_current_region(root, priority);
            if let Ok(region) = &generated {
                authority.lock_cache().insert(Arc::clone(region));
            }
            authority.finish_flight(root, &flight);
            generated
        })
        .await
        .map_err(|_| VirtualTerrainError::TaskFailed)?
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
        if (1..=TERRAIN_COVERAGE_ROOT_LEVEL).contains(&root.level) && root.is_surface() {
            for _ in 0..REGION_BUILD_ATTEMPTS {
                let snapshot = self
                    .edits
                    .snapshot_surface_terrain(root)
                    .ok_or(VirtualTerrainError::InvalidRoot)?;
                let built = build_coverage_region(
                    self.source.as_ref(),
                    root,
                    snapshot.clone(),
                    self.source_identity_hash(),
                    priority,
                    |key| self.edits.surface_terrain_revision(key),
                )?;
                if self.edits.surface_terrain_revision(root) == Some(snapshot.revision) {
                    return prepare_region(built);
                }
            }
            return Err(VirtualTerrainError::ChangedDuringBuild);
        }
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
        _priority: WorldProductPriority,
    ) -> Result<PreparedTerrainRegionColumn, VirtualTerrainError> {
        let horizontal_root =
            TerrainPageKey::surface(TERRAIN_COVERAGE_ROOT_LEVEL, column[0], column[1]);
        horizontal_root
            .horizontal_bounds()
            .ok_or(VirtualTerrainError::InvalidRoot)?;
        let revision = self.edits.revision();
        let roots = vec![horizontal_root];
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

fn build_coverage_region(
    source: &dyn WorldSourceEngine,
    root: TerrainPageKey,
    snapshot: TerrainEditSnapshot,
    source_identity_hash: WorldSourceIdentityHash,
    priority: WorldProductPriority,
    mut revision_at: impl FnMut(TerrainPageKey) -> Option<u64>,
) -> Result<TerrainRegionBuildV1, VirtualTerrainError> {
    if root.level == 0 || root.level > TERRAIN_COVERAGE_ROOT_LEVEL || !root.is_surface() {
        return Err(VirtualTerrainError::InvalidRoot);
    }
    let [[minimum_x, minimum_z], _] = root
        .horizontal_bounds()
        .ok_or(VirtualTerrainError::InvalidRoot)?;
    let stride = 1u32
        .checked_shl(u32::from(root.level.saturating_sub(1)))
        .ok_or(VirtualTerrainError::InvalidRoot)?;
    let samples = source
        .surface_sample_lattice(
            priority,
            [minimum_x, minimum_z],
            [TERRAIN_PAGE_EDGE_SAMPLES * 2 + 1; 2],
            stride,
        )
        .map_err(|error| VirtualTerrainError::Source(error.to_string()))?;
    let child_boundary_midpoints =
        sample_child_heightfield_boundary_midpoints(source, root, priority)?;
    let has_edits = surface_page_has_edits(&snapshot, root);
    let exact_scan = if root.level == 1 && has_edits {
        Some(sample_exact_surface_edits(
            source, root, &samples, &snapshot, priority,
        )?)
    } else {
        None
    };
    let mut built = build_terrain_coverage_root_with_revisions(
        source_identity_hash,
        root,
        &mut revision_at,
        |key| TerrainErrorBounds {
            geometric_millivoxels: 1_000_u32
                .checked_shl(u32::from(key.level))
                .unwrap_or(u32::MAX),
            silhouette_millivoxels: 1_000_u32
                .checked_shl(u32::from(key.level))
                .unwrap_or(u32::MAX),
            material_boundary_millivoxels: 0,
            normal_milliradians: 0,
            // Topology uncertainty belongs to the directional surface owner whose interior or
            // positive handoff sample intersects an edit. It must not depend on which larger
            // directory happened to reveal the page.
            unresolved_topology: surface_page_has_edits(&snapshot, key),
        },
        &samples,
        &child_boundary_midpoints,
    )
    .map_err(|error| VirtualTerrainError::Build(error.to_string()))?;
    if let Some(exact_scan) = exact_scan {
        for child in root
            .refinement_children()
            .ok_or(VirtualTerrainError::InvalidRoot)?
        {
            let revision = revision_at(child).ok_or(VirtualTerrainError::InvalidRoot)?;
            let page = build_exact_surface_terrain_page(
                source_identity_hash,
                child,
                revision,
                exact_scan.vertical_bounds,
                |coord| exact_scan.sample(coord),
            )
            .map_err(|error| VirtualTerrainError::Build(error.to_string()))?;
            let encoded_bytes = encode_terrain_page(&page)
                .map_err(|error| VirtualTerrainError::Codec(error.to_string()))?
                .len();
            if encoded_bytes > TERRAIN_PAGE_TARGET_COMPRESSED_BYTES {
                return Err(VirtualTerrainError::Build(format!(
                    "exact surface page {child:?} is {encoded_bytes} bytes, above the publication budget"
                )));
            }
            let Some(existing) = built
                .pages
                .iter_mut()
                .find(|candidate| candidate.key == child)
            else {
                return Err(VirtualTerrainError::Build(format!(
                    "surface refinement omitted exact child {child:?}"
                )));
            };
            *existing = page;
        }
        built.directory =
            TerrainHierarchyDirectoryV1::from_surface_refinement_pages(root, &built.pages)
                .map_err(|error| VirtualTerrainError::Build(error.to_string()))?;
    }
    Ok(built)
}

fn surface_page_has_edits(snapshot: &TerrainEditSnapshot, key: TerrainPageKey) -> bool {
    let Some([[minimum_x, minimum_z], [maximum_x, maximum_z]]) = key.horizontal_bounds() else {
        return false;
    };
    snapshot.edits.edited_chunks().into_iter().any(|chunk| {
        snapshot
            .edits
            .chunk_overrides(chunk)
            .into_iter()
            .any(|(coord, _)| {
                (minimum_x..=maximum_x).contains(&coord.x)
                    && (minimum_z..=maximum_z).contains(&coord.z)
            })
    })
}

fn sample_child_heightfield_boundary_midpoints(
    source: &dyn WorldSourceEngine,
    root: TerrainPageKey,
    priority: WorldProductPriority,
) -> Result<BTreeMap<TerrainPageKey, Vec<voxels_world::SurfaceSample>>, VirtualTerrainError> {
    if root.level <= 1 {
        return Ok(BTreeMap::new());
    }
    let [[minimum_x, minimum_z], [maximum_x, maximum_z]] = root
        .horizontal_bounds()
        .ok_or(VirtualTerrainError::InvalidRoot)?;
    let child_stride = 1u32
        .checked_shl(u32::from(root.level - 1))
        .ok_or(VirtualTerrainError::InvalidRoot)?;
    let midpoint_offset =
        i32::try_from(child_stride / 2).map_err(|_| VirtualTerrainError::InvalidRoot)?;
    let half_x = minimum_x + (maximum_x - minimum_x) / 2;
    let half_z = minimum_z + (maximum_z - minimum_z) / 2;
    let vertical = [minimum_x, half_x, maximum_x]
        .map(|x| {
            source
                .surface_sample_lattice(
                    priority,
                    [x, minimum_z.saturating_add(midpoint_offset)],
                    [1, TERRAIN_PAGE_EDGE_SAMPLES * 2],
                    child_stride,
                )
                .map_err(|error| VirtualTerrainError::Source(error.to_string()))
        })
        .into_iter()
        .collect::<Result<Vec<_>, _>>()?;
    let horizontal = [minimum_z, half_z, maximum_z]
        .map(|z| {
            source
                .surface_sample_lattice(
                    priority,
                    [minimum_x.saturating_add(midpoint_offset), z],
                    [TERRAIN_PAGE_EDGE_SAMPLES * 2, 1],
                    child_stride,
                )
                .map_err(|error| VirtualTerrainError::Source(error.to_string()))
        })
        .into_iter()
        .collect::<Result<Vec<_>, _>>()?;
    let side_samples = TERRAIN_PAGE_EDGE_SAMPLES as usize;
    root.refinement_children()
        .ok_or(VirtualTerrainError::InvalidRoot)?
        .into_iter()
        .enumerate()
        .map(|(index, key)| {
            let quadrant_x = index & 1;
            let quadrant_z = (index >> 1) & 1;
            let tangent_x = quadrant_x * side_samples;
            let tangent_z = quadrant_z * side_samples;
            let mut residuals =
                Vec::with_capacity(voxels_world::TERRAIN_HEIGHTFIELD_BOUNDARY_MIDPOINTS);
            residuals.extend_from_slice(&vertical[quadrant_x][tangent_z..tangent_z + side_samples]);
            residuals
                .extend_from_slice(&vertical[quadrant_x + 1][tangent_z..tangent_z + side_samples]);
            residuals
                .extend_from_slice(&horizontal[quadrant_z][tangent_x..tangent_x + side_samples]);
            residuals.extend_from_slice(
                &horizontal[quadrant_z + 1][tangent_x..tangent_x + side_samples],
            );
            Ok((key, residuals))
        })
        .collect()
}

struct ExactSurfaceScan {
    minimum: VoxelCoord,
    shape: [usize; 3],
    vertical_bounds: [i32; 2],
    materials: Vec<Material>,
}

impl ExactSurfaceScan {
    fn sample(&self, coord: VoxelCoord) -> Material {
        let local = [
            i64::from(coord.x) - i64::from(self.minimum.x),
            i64::from(coord.y) - i64::from(self.minimum.y),
            i64::from(coord.z) - i64::from(self.minimum.z),
        ];
        if local.iter().zip(self.shape).any(|(component, shape)| {
            *component < 0 || *component >= i64::try_from(shape).unwrap_or(i64::MAX)
        }) {
            return Material::Air;
        }
        let [x, y, z] = local.map(|component| component as usize);
        self.materials[x + y * self.shape[0] + z * self.shape[0] * self.shape[1]]
    }
}

fn sample_exact_surface_edits(
    source: &dyn WorldSourceEngine,
    root: TerrainPageKey,
    samples: &[voxels_world::SurfaceSample],
    snapshot: &TerrainEditSnapshot,
    priority: WorldProductPriority,
) -> Result<ExactSurfaceScan, VirtualTerrainError> {
    const SAMPLE_EDGE: usize = (TERRAIN_PAGE_EDGE_SAMPLES as usize) * 2 + 1;
    const EXACT_SAMPLE_EDGE: usize = SAMPLE_EDGE + 1;
    const REQUEST_BATCH: usize = 8;
    if samples.len() != SAMPLE_EDGE * SAMPLE_EDGE {
        return Err(VirtualTerrainError::Build(
            "edited surface lattice has the wrong shape".to_owned(),
        ));
    }
    let [[minimum_x, minimum_z], [maximum_x, maximum_z]] = root
        .horizontal_bounds()
        .ok_or(VirtualTerrainError::InvalidRoot)?;
    let mut edited_columns = BTreeSet::new();
    let mut edited_bounds = BTreeMap::<voxels_world::ChunkCoord, [VoxelCoord; 2]>::new();
    let mut minimum_y = samples
        .iter()
        .map(|sample| sample.height)
        .min()
        .ok_or_else(|| VirtualTerrainError::Build("edited surface lattice is empty".to_owned()))?;
    let mut maximum_y = samples
        .iter()
        .map(|sample| sample.water_level.unwrap_or(sample.height))
        .max()
        .ok_or_else(|| VirtualTerrainError::Build("edited surface lattice is empty".to_owned()))?;
    for chunk in snapshot.edits.edited_chunks() {
        for (coord, _) in snapshot.edits.chunk_overrides(chunk) {
            if (minimum_x..=maximum_x).contains(&coord.x)
                && (minimum_z..=maximum_z).contains(&coord.z)
            {
                edited_columns.insert((coord.x, coord.z));
                edited_bounds
                    .entry(chunk)
                    .and_modify(|bounds| {
                        bounds[0].x = bounds[0].x.min(coord.x);
                        bounds[0].y = bounds[0].y.min(coord.y);
                        bounds[0].z = bounds[0].z.min(coord.z);
                        bounds[1].x = bounds[1].x.max(coord.x);
                        bounds[1].y = bounds[1].y.max(coord.y);
                        bounds[1].z = bounds[1].z.max(coord.z);
                    })
                    .or_insert([coord, coord]);
                minimum_y = minimum_y.min(coord.y);
                maximum_y = maximum_y.max(coord.y);
            }
        }
    }
    if edited_columns.is_empty() {
        return Err(VirtualTerrainError::Build(
            "edited surface snapshot contained no columns in its root".to_owned(),
        ));
    }
    minimum_y = minimum_y
        .checked_sub(1)
        .ok_or(VirtualTerrainError::InvalidRoot)?;
    maximum_y = maximum_y
        .checked_add(FEATURE_MAX_RADIUS_VOXELS + 2)
        .ok_or(VirtualTerrainError::InvalidRoot)?;
    let vertical_bounds = [minimum_y, maximum_y];
    let sample_minimum = VoxelCoord::new(
        minimum_x
            .checked_sub(1)
            .ok_or(VirtualTerrainError::InvalidRoot)?,
        minimum_y
            .checked_sub(1)
            .ok_or(VirtualTerrainError::InvalidRoot)?,
        minimum_z
            .checked_sub(1)
            .ok_or(VirtualTerrainError::InvalidRoot)?,
    );
    let sample_maximum_y = maximum_y
        .checked_add(1)
        .ok_or(VirtualTerrainError::InvalidRoot)?;
    let shape_y = usize::try_from(i64::from(sample_maximum_y) - i64::from(sample_minimum.y))
        .map_err(|_| VirtualTerrainError::InvalidRoot)?;
    let shape = [EXACT_SAMPLE_EDGE, shape_y, EXACT_SAMPLE_EDGE];
    let sample_count = shape
        .into_iter()
        .try_fold(1usize, usize::checked_mul)
        .ok_or(VirtualTerrainError::InvalidRoot)?;
    let mut exact = ExactSurfaceScan {
        minimum: sample_minimum,
        shape,
        vertical_bounds,
        materials: vec![Material::Air; sample_count],
    };

    // The ordinary surface predictor already certifies the unedited part of this page. Populate
    // that complete, hole-free owner first, then replace only the small neighborhoods that can
    // expose canonical material through an edit. Resampling the whole 66 x N x 66 volume made a
    // four-voxel dig wait tens of seconds on unrelated procedural samples.
    let predictor = source
        .surface_sample_lattice(
            priority,
            [sample_minimum.x, sample_minimum.z],
            [EXACT_SAMPLE_EDGE as u32; 2],
            1,
        )
        .map_err(|error| VirtualTerrainError::Source(error.to_string()))?;
    if predictor.len() != EXACT_SAMPLE_EDGE * EXACT_SAMPLE_EDGE {
        return Err(VirtualTerrainError::Source(
            "source returned a mismatched edited surface predictor".to_owned(),
        ));
    }
    for z in 0..EXACT_SAMPLE_EDGE {
        for y in 0..shape_y {
            let world_y = sample_minimum.y + y as i32;
            for x in 0..EXACT_SAMPLE_EDGE {
                let column = predictor[x + z * EXACT_SAMPLE_EDGE];
                let material = if world_y <= column.height {
                    column.material
                } else if column
                    .water_level
                    .is_some_and(|water_level| world_y <= water_level)
                {
                    Material::Water
                } else {
                    Material::Air
                };
                exact.materials[x + y * exact.shape[0] + z * exact.shape[0] * exact.shape[1]] =
                    material;
            }
        }
    }

    let sample_maximum = VoxelCoord::new(
        sample_minimum
            .x
            .checked_add(i32::try_from(shape[0]).map_err(|_| VirtualTerrainError::InvalidRoot)?)
            .ok_or(VirtualTerrainError::InvalidRoot)?,
        sample_maximum_y,
        sample_minimum
            .z
            .checked_add(i32::try_from(shape[2]).map_err(|_| VirtualTerrainError::InvalidRoot)?)
            .ok_or(VirtualTerrainError::InvalidRoot)?,
    );
    let mut requests = edited_bounds
        .into_values()
        .map(|[minimum, maximum]| {
            let minimum = VoxelCoord::new(
                minimum.x.saturating_sub(1).max(sample_minimum.x),
                minimum.y.saturating_sub(1).max(sample_minimum.y),
                minimum.z.saturating_sub(1).max(sample_minimum.z),
            );
            let maximum = VoxelCoord::new(
                maximum.x.saturating_add(2).min(sample_maximum.x),
                maximum.y.saturating_add(2).min(sample_maximum.y),
                maximum.z.saturating_add(2).min(sample_maximum.z),
            );
            let shape = [
                u32::try_from(maximum.x - minimum.x),
                u32::try_from(maximum.y - minimum.y),
                u32::try_from(maximum.z - minimum.z),
            ];
            shape
                .map(|component| component.map_err(|_| VirtualTerrainError::InvalidRoot))
                .into_iter()
                .collect::<Result<Vec<_>, _>>()
                .and_then(|shape| {
                    let [x, y, z]: [u32; 3] = shape
                        .try_into()
                        .map_err(|_| VirtualTerrainError::InvalidRoot)?;
                    Ok(VoxelBlockRequest {
                        min: minimum,
                        sample_shape: [x, y, z],
                    })
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    requests.sort_unstable_by_key(|request| request.min);
    for batch in requests.chunks(REQUEST_BATCH) {
        let requested = batch
            .iter()
            .copied()
            .map(WorldProductRequest::VoxelBlock)
            .collect::<Vec<_>>();
        let result = source
            .generate_batch(WorldProductBatch {
                priority,
                requests: requested.clone(),
            })
            .map_err(|error| VirtualTerrainError::Source(error.to_string()))?;
        if result.source_identity_hash != source.identity().identity_hash()
            || result.items.len() != requested.len()
        {
            return Err(VirtualTerrainError::Source(
                "source returned a mismatched edited surface batch".to_owned(),
            ));
        }
        for (expected, item) in requested.into_iter().zip(result.items) {
            let WorldProductRequest::VoxelBlock(request) = expected else {
                unreachable!("edited surface requests contain only voxel blocks");
            };
            let snapshot_block = match (item.request, item.result) {
                (
                    WorldProductRequest::VoxelBlock(returned),
                    Ok(WorldProduct::VoxelBlock(block)),
                ) if returned == request && block.request == request => block,
                (_, Err(error)) => {
                    return Err(VirtualTerrainError::Source(error.to_string()));
                }
                _ => {
                    return Err(VirtualTerrainError::Source(
                        "source returned a mismatched edited surface block".to_owned(),
                    ));
                }
            };
            let shape_x = request.sample_shape[0] as usize;
            let shape_y = request.sample_shape[1] as usize;
            let shape_z = request.sample_shape[2] as usize;
            for z in 0..shape_z {
                let world_z = request.min.z + z as i32;
                for y in 0..shape_y {
                    let world_y = request.min.y + y as i32;
                    for x in 0..shape_x {
                        let world_x = request.min.x + x as i32;
                        let source_index = x + y * shape_x + z * shape_x * shape_y;
                        let generated = snapshot_block
                            .materials()
                            .get(source_index)
                            .copied()
                            .ok_or_else(|| {
                                VirtualTerrainError::Source(
                                    "edited surface block omitted a sample".to_owned(),
                                )
                            })?;
                        let material = snapshot.edits.resolve_generated(
                            VoxelCoord::new(world_x, world_y, world_z),
                            generated,
                        );
                        let exact_x = usize::try_from(world_x - exact.minimum.x)
                            .map_err(|_| VirtualTerrainError::InvalidRoot)?;
                        let exact_y = usize::try_from(world_y - exact.minimum.y)
                            .map_err(|_| VirtualTerrainError::InvalidRoot)?;
                        let exact_z = usize::try_from(world_z - exact.minimum.z)
                            .map_err(|_| VirtualTerrainError::InvalidRoot)?;
                        let exact_index = exact_x
                            + exact_y * exact.shape[0]
                            + exact_z * exact.shape[0] * exact.shape[1];
                        let Some(destination) = exact.materials.get_mut(exact_index) else {
                            return Err(VirtualTerrainError::Source(
                                "edited surface block exceeded its requested owner".to_owned(),
                            ));
                        };
                        *destination = material;
                    }
                }
            }
        }
    }

    Ok(exact)
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
        .node(built.root)
        .map(|node| node.revision)
        .ok_or_else(|| VirtualTerrainError::Build("terrain directory omits its root".to_owned()))?;
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
            sample_region(&source, root(), WorldProductPriority::VisibleChunk).expect("sample");
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

    #[test]
    fn exact_surface_edit_discovery_matches_directional_page_ownership() {
        let key = TerrainPageKey::surface(1, -2, 3);
        let [[minimum_x, minimum_z], [maximum_x, maximum_z]] =
            key.horizontal_bounds().unwrap();
        let snapshot_at = |x, z| {
            let mut edits = voxels_world::EditMap::default();
            edits.insert_override(VoxelCoord::new(x, 0, z), Material::Basalt);
            TerrainEditSnapshot { edits, revision: 2 }
        };

        assert!(surface_page_has_edits(
            &snapshot_at(minimum_x, minimum_z),
            key
        ));
        assert!(surface_page_has_edits(
            &snapshot_at(maximum_x, maximum_z),
            key
        ));
        assert!(
            !surface_page_has_edits(&snapshot_at(minimum_x - 1, minimum_z), key),
            "the negative neighbor owns its own interface edit"
        );
        assert!(
            !surface_page_has_edits(&snapshot_at(maximum_x + 1, maximum_z), key),
            "only the positive handoff sample belongs to this page"
        );
    }

    #[test]
    fn coverage_root_is_an_independent_refinable_heightfield_group() {
        let source = ProceduralWorldSource::new(17);
        let root = TerrainPageKey::surface(TERRAIN_COVERAGE_ROOT_LEVEL, -1, 1);
        let built = build_coverage_region(
            &source,
            root,
            TerrainEditSnapshot {
                edits: voxels_world::EditMap::default(),
                revision: 9,
            },
            source.source_identity_hash(),
            WorldProductPriority::VirtualTerrain,
            |_| Some(9),
        )
        .expect("coverage");
        assert_eq!(built.pages.len(), 5);
        assert_eq!(built.directory.nodes.len(), 5);
        assert_eq!(built.directory.roots().next().unwrap().key, root);
        assert!(built.pages.iter().all(|page| matches!(
            page.representation,
            voxels_world::TerrainPageRepresentation::HeightfieldGrid(_)
        )));
        assert!(built.pages.iter().all(|page| {
            encode_terrain_page(page).unwrap().len() <= TERRAIN_PAGE_TARGET_COMPRESSED_BYTES
        }));
    }

    #[test]
    fn surface_page_identity_is_independent_of_directory_discovery_depth() {
        let source = ProceduralWorldSource::new(17);
        let parent_root = TerrainPageKey::surface(TERRAIN_COVERAGE_ROOT_LEVEL, -1, 1);
        let child_root = parent_root.refinement_children().unwrap()[0];
        let parent = build_coverage_region(
            &source,
            parent_root,
            TerrainEditSnapshot {
                edits: voxels_world::EditMap::default(),
                revision: 9,
            },
            source.source_identity_hash(),
            WorldProductPriority::VirtualTerrain,
            |key| Some(if key == child_root { 13 } else { 23 }),
        )
        .expect("parent coverage");
        let child = build_coverage_region(
            &source,
            child_root,
            TerrainEditSnapshot {
                edits: voxels_world::EditMap::default(),
                revision: 9,
            },
            source.source_identity_hash(),
            WorldProductPriority::VirtualTerrain,
            |key| Some(if key == child_root { 13 } else { 29 }),
        )
        .expect("child coverage");

        let embedded = parent
            .pages
            .iter()
            .find(|page| page.key == child_root)
            .expect("embedded child");
        let independent = child
            .pages
            .iter()
            .find(|page| page.key == child_root)
            .expect("independent child");
        assert_eq!(independent, embedded);
    }

    #[test]
    fn edited_surface_page_identity_is_independent_of_discovery_depth() {
        let source = ProceduralWorldSource::new(17);
        let parent_root = TerrainPageKey::surface(2, -1, 0);
        let child_root = parent_root.refinement_children().unwrap()[1];
        let [[minimum_x, minimum_z], _] = child_root.horizontal_bounds().unwrap();

        for tangent_offset in [5, 6] {
            let surface = source
                .surface_sample_lattice(
                    WorldProductPriority::VirtualTerrain,
                    [minimum_x, minimum_z + tangent_offset],
                    [1, 1],
                    1,
                )
                .unwrap()[0];
            let mut edits = voxels_world::EditMap::default();
            let material = if tangent_offset % 2 == 0 {
                Material::Basalt
            } else {
                Material::Air
            };
            edits.insert_override(
                VoxelCoord::new(minimum_x, surface.height, minimum_z + tangent_offset),
                material,
            );
            let snapshot = TerrainEditSnapshot {
                edits,
                revision: 31,
            };
            let parent = build_coverage_region(
                &source,
                parent_root,
                snapshot.clone(),
                source.source_identity_hash(),
                WorldProductPriority::VirtualTerrain,
                |key| Some(if key == child_root { 37 } else { 31 }),
            )
            .expect("parent coverage");
            let child = build_coverage_region(
                &source,
                child_root,
                snapshot,
                source.source_identity_hash(),
                WorldProductPriority::VirtualTerrain,
                |key| Some(if key == child_root { 37 } else { 31 }),
            )
            .expect("child coverage");

            let embedded = parent
                .pages
                .iter()
                .find(|page| page.key == child_root)
                .expect("embedded child");
            let independent = child
                .pages
                .iter()
                .find(|page| page.key == child_root)
                .expect("independent child");
            assert_eq!(
                independent, embedded,
                "an outer-edge edit at {tangent_offset} changed L1 identity with discovery depth"
            );
        }
    }

    #[test]
    fn an_edit_does_not_change_unaffected_sibling_identity() {
        let source = ProceduralWorldSource::new(17);
        let parent_root = TerrainPageKey::surface(2, 0, -1);
        let children = parent_root.refinement_children().unwrap();
        let edited_root = children[0];
        let untouched_root = children[3];
        let [[minimum_x, minimum_z], [maximum_x, maximum_z]] =
            edited_root.horizontal_bounds().unwrap();
        let [edit_x, edit_z] = [
            minimum_x + (maximum_x - minimum_x) / 2,
            minimum_z + (maximum_z - minimum_z) / 2,
        ];
        let surface = source
            .surface_sample_lattice(
                WorldProductPriority::VirtualTerrain,
                [edit_x, edit_z],
                [1, 1],
                1,
            )
            .unwrap()[0];
        let mut edits = voxels_world::EditMap::default();
        edits.insert_override(
            VoxelCoord::new(edit_x, surface.height, edit_z),
            Material::Air,
        );
        let snapshot = TerrainEditSnapshot {
            edits,
            revision: 43,
        };
        let revision_at = |key| Some(if key == untouched_root { 47 } else { 43 });
        let parent = build_coverage_region(
            &source,
            parent_root,
            snapshot.clone(),
            source.source_identity_hash(),
            WorldProductPriority::VirtualTerrain,
            revision_at,
        )
        .expect("parent coverage");
        let child = build_coverage_region(
            &source,
            untouched_root,
            snapshot,
            source.source_identity_hash(),
            WorldProductPriority::VirtualTerrain,
            revision_at,
        )
        .expect("unaffected child coverage");

        let embedded = parent
            .pages
            .iter()
            .find(|page| page.key == untouched_root)
            .expect("embedded unaffected child");
        let independent = child
            .pages
            .iter()
            .find(|page| page.key == untouched_root)
            .expect("independent unaffected child");
        assert!(!embedded.errors.unresolved_topology);
        assert_eq!(independent, embedded);
    }

    #[test]
    fn prepared_surface_segment_keeps_page_local_revisions() {
        let source = ProceduralWorldSource::new(17);
        let root = TerrainPageKey::surface(1, 0, 0);
        let revised_child = root.refinement_children().unwrap()[2];
        let built = build_coverage_region(
            &source,
            root,
            TerrainEditSnapshot {
                edits: voxels_world::EditMap::default(),
                revision: 41,
            },
            source.source_identity_hash(),
            WorldProductPriority::VirtualTerrain,
            |key| Some(if key == revised_child { 43 } else { 41 }),
        )
        .expect("coverage");
        let prepared = prepare_region(built).expect("prepared mixed-revision segment");
        assert_eq!(prepared.revision, 41);
        let child_node = prepared.directory.node(revised_child).unwrap();
        assert_eq!(child_node.revision, 43);
        let child = prepared
            .page(TerrainPageTransferIdentity {
                key: revised_child,
                revision: child_node.revision,
                content_fingerprint: child_node.content_fingerprint,
            })
            .unwrap()
            .unwrap();
        assert_eq!(child.revision, 43);
    }

    #[test]
    fn finest_surface_segment_applies_the_local_edit_snapshot() {
        let source = ProceduralWorldSource::new(17);
        let root = TerrainPageKey::surface(1, 0, 0);
        let [[minimum_x, minimum_z], _] = root.horizontal_bounds().unwrap();
        let pristine = source
            .surface_sample_lattice(
                WorldProductPriority::VirtualTerrain,
                [minimum_x, minimum_z],
                [TERRAIN_PAGE_EDGE_SAMPLES * 2 + 1; 2],
                1,
            )
            .unwrap();
        let edge = TERRAIN_PAGE_EDGE_SAMPLES as usize * 2 + 1;
        let [local_x, local_z] = [3usize, 4usize];
        let baseline = pristine[local_x + local_z * edge];
        let edited_coord = VoxelCoord::new(
            minimum_x + local_x as i32,
            baseline.height + 5,
            minimum_z + local_z as i32,
        );
        let mut edits = voxels_world::EditMap::default();
        edits.insert_override(edited_coord, Material::Basalt);
        let built = build_coverage_region(
            &source,
            root,
            TerrainEditSnapshot {
                edits,
                revision: 11,
            },
            source.source_identity_hash(),
            WorldProductPriority::VirtualTerrain,
            |_| Some(11),
        )
        .unwrap();
        let child = built
            .pages
            .iter()
            .find(|page| page.key == TerrainPageKey::surface(0, 0, 0))
            .unwrap();
        let voxels_world::TerrainPageRepresentation::SurfaceCluster(quads) = &child.representation
        else {
            panic!("edited finest surface child is not an exact cluster");
        };
        assert!(quads.iter().any(|quad| {
            quad.axis == voxels_world::FaceAxis::Y
                && quad.positive
                && quad.plane == edited_coord.y + 1
                && (quad.u..quad.u + i32::from(quad.width)).contains(&edited_coord.x)
                && (quad.v..quad.v + i32::from(quad.height)).contains(&edited_coord.z)
                && child.materials[usize::from(quad.material_index)].material == Material::Basalt
        }));
        assert_eq!(child.errors, TerrainErrorBounds::EXACT);
        assert_eq!(
            child.topology,
            voxels_world::TerrainTopologyClass::Volumetric
        );

        let parent = built.pages.iter().find(|page| page.key == root).unwrap();
        assert!(parent.errors.unresolved_topology);
        let children = root
            .refinement_children()
            .unwrap()
            .into_iter()
            .map(|key| {
                built
                    .pages
                    .iter()
                    .find(|page| page.key == key)
                    .unwrap()
                    .clone()
            })
            .collect::<Vec<_>>();
        voxels_world::validate_terrain_replacement(parent, &children).unwrap();
    }
}
