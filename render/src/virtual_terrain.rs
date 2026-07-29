//! Bounded resident hierarchy and one-owner cut selection for virtual microvoxel terrain.
//!
//! This module is deliberately independent of WGPU resources. It is the CPU oracle for the GPU
//! traversal: roots remain valid fallbacks, refinement replaces a parent only as a complete
//! certified child group, and request feedback is bounded without ever manufacturing geometry.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::Arc;
use voxels_world::{
    TERRAIN_COVERAGE_ROOT_LEVEL, TERRAIN_PAGE_EDGE_SAMPLES, TERRAIN_PAGE_MAX_CHILDREN,
    TerrainHierarchyDirectoryV1, TerrainHierarchyNode, TerrainPageKey, TerrainPageRepresentation,
    TerrainPageTransferIdentity, TerrainPageV1, WorldSourceIdentityHash, encode_terrain_page,
    reconstruct_exact_terrain_surface, validate_terrain_replacement,
};

const FINGERPRINT_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FINGERPRINT_PRIME: u64 = 0x0000_0100_0000_01b3;
const NORMAL_ERROR_PIXELS_PER_RADIAN: f64 = 0.25;
const PRESENTATION_LOCUS_EDGE_LEAVES: i32 = 4;
const PRESENTATION_LOCUS_STRIDE_LEAVES: i32 = 2;

/// A snapped, overlapping horizontal region in which a committed terrain presentation is safe.
///
/// Adjacent loci overlap by two level-0 pages. This lets a replacement promote while the camera
/// is still valid in both the old and new presentation; the simulation only has to wait at the
/// outer half-open boundary when publication loses the race.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct PresentationLocus {
    minimum_leaf: [i32; 2],
    maximum_leaf_exclusive: [i32; 2],
}

impl PresentationLocus {
    pub fn containing_position(position_metres: [f64; 3]) -> Option<Self> {
        if !position_metres.into_iter().all(f64::is_finite) {
            return None;
        }
        let leaf_span_metres = f64::from(TERRAIN_PAGE_EDGE_SAMPLES) * 0.1;
        let camera_leaf = [position_metres[0], position_metres[2]].map(|coordinate| {
            let leaf = (coordinate / leaf_span_metres).floor();
            (leaf >= f64::from(i32::MIN) && leaf <= f64::from(i32::MAX)).then_some(leaf as i32)
        });
        let [Some(camera_leaf_x), Some(camera_leaf_z)] = camera_leaf else {
            return None;
        };
        let anchor = [camera_leaf_x, camera_leaf_z].map(|coordinate| {
            coordinate.div_euclid(PRESENTATION_LOCUS_STRIDE_LEAVES)
                * PRESENTATION_LOCUS_STRIDE_LEAVES
        });
        let minimum_leaf = [anchor[0].checked_sub(1)?, anchor[1].checked_sub(1)?];
        let maximum_leaf_exclusive = [
            anchor[0].checked_add(PRESENTATION_LOCUS_EDGE_LEAVES - 1)?,
            anchor[1].checked_add(PRESENTATION_LOCUS_EDGE_LEAVES - 1)?,
        ];
        Some(Self {
            minimum_leaf,
            maximum_leaf_exclusive,
        })
    }

    pub const fn minimum_leaf(self) -> [i32; 2] {
        self.minimum_leaf
    }

    pub const fn maximum_leaf_exclusive(self) -> [i32; 2] {
        self.maximum_leaf_exclusive
    }

    pub fn horizontal_bounds_metres(self) -> [[f64; 2]; 2] {
        let leaf_span_metres = f64::from(TERRAIN_PAGE_EDGE_SAMPLES) * 0.1;
        [
            self.minimum_leaf
                .map(|coordinate| f64::from(coordinate) * leaf_span_metres),
            self.maximum_leaf_exclusive
                .map(|coordinate| f64::from(coordinate) * leaf_span_metres),
        ]
    }

    /// Half-open membership used by both simulation gating and presentation diagnostics.
    pub fn contains_position(self, position_metres: [f32; 3]) -> bool {
        if !position_metres.into_iter().all(f32::is_finite) {
            return false;
        }
        let [minimum, maximum] = self.horizontal_bounds_metres();
        let position = [position_metres[0], position_metres[2]];
        (minimum[0] as f32..maximum[0] as f32).contains(&position[0])
            && (minimum[1] as f32..maximum[1] as f32).contains(&position[1])
    }

    /// Clamps horizontal motion to this convex half-open locus while preserving vertical motion.
    ///
    /// The maximum representable `f32` below the exclusive edge is used instead of an arbitrary
    /// epsilon, so repeated blocked steps cannot drift outside or leave a visible dead strip.
    pub fn clamp_position(self, mut position_metres: [f32; 3]) -> [f32; 3] {
        let [minimum, maximum] = self.horizontal_bounds_metres();
        for (world_axis, horizontal_axis) in [(0, 0), (2, 1)] {
            let minimum = minimum[horizontal_axis] as f32;
            let maximum = next_down_f32(maximum[horizontal_axis] as f32);
            position_metres[world_axis] = position_metres[world_axis].clamp(minimum, maximum);
        }
        position_metres
    }
}

fn next_down_f32(value: f32) -> f32 {
    if value.is_nan() || value == f32::NEG_INFINITY {
        return value;
    }
    if value == 0.0 {
        return -f32::from_bits(1);
    }
    let bits = value.to_bits();
    f32::from_bits(if value > 0.0 { bits - 1 } else { bits + 1 })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PresentationEnvelopeInput {
    locus: PresentationLocus,
    exact_radius_bits: u64,
    view_distance_bits: u64,
    max_exact_leaves: usize,
    max_horizon_roots: usize,
}

/// Immutable reuse while the desired camera remains in one snapped locus.
#[derive(Clone, Debug, Default)]
pub struct PresentationEnvelopeCache {
    entry: Option<(PresentationEnvelopeInput, PresentationEnvelope)>,
    rebuilds: u64,
}

impl PresentationEnvelopeCache {
    pub fn resolve(
        &mut self,
        camera_position_metres: [f64; 3],
        exact_radius_metres: f64,
        view_distance_metres: f64,
        max_exact_leaves: usize,
        max_horizon_roots: usize,
    ) -> PresentationEnvelope {
        let Some(locus) = PresentationLocus::containing_position(camera_position_metres) else {
            return PresentationEnvelope::incomplete();
        };
        let input = PresentationEnvelopeInput {
            locus,
            exact_radius_bits: exact_radius_metres.to_bits(),
            view_distance_bits: view_distance_metres.to_bits(),
            max_exact_leaves,
            max_horizon_roots,
        };
        if let Some((cached_input, envelope)) = &self.entry
            && *cached_input == input
        {
            return envelope.clone();
        }
        let envelope = PresentationEnvelope::build(
            locus,
            exact_radius_metres,
            view_distance_metres,
            max_exact_leaves,
            max_horizon_roots,
        );
        self.rebuilds = self.rebuilds.saturating_add(1);
        self.entry = Some((input, envelope.clone()));
        envelope
    }

    pub const fn rebuilds(&self) -> u64 {
        self.rebuilds
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PresentationEnvelope {
    data: Arc<PresentationEnvelopeData>,
}

#[derive(Debug, Eq, PartialEq)]
struct PresentationEnvelopeData {
    locus: Option<PresentationLocus>,
    exact_surface_domain: ExactSurfaceDomain,
    horizon_roots: BTreeSet<TerrainPageKey>,
    exact_radius_bits: u64,
    view_distance_bits: u64,
    max_exact_leaves: usize,
    max_horizon_roots: usize,
    complete: bool,
    fingerprint: u64,
}

impl PresentationEnvelope {
    fn build(
        locus: PresentationLocus,
        exact_radius_metres: f64,
        view_distance_metres: f64,
        max_exact_leaves: usize,
        max_horizon_roots: usize,
    ) -> Self {
        if !exact_radius_metres.is_finite()
            || exact_radius_metres < 0.0
            || !view_distance_metres.is_finite()
            || view_distance_metres <= 0.0
        {
            return Self::incomplete();
        }
        let Some(exact_leaves) =
            enumerate_locus_dilation_pages(locus, 0, exact_radius_metres, max_exact_leaves)
        else {
            return Self::incomplete();
        };
        let Some(horizon_roots) = enumerate_locus_dilation_pages(
            locus,
            TERRAIN_COVERAGE_ROOT_LEVEL,
            view_distance_metres,
            max_horizon_roots,
        ) else {
            return Self::incomplete();
        };
        let exact_surface_domain =
            ExactSurfaceDomain::from_parts(exact_leaves.clone(), Some(exact_leaves));
        if !exact_surface_domain.is_complete() || horizon_roots.is_empty() {
            return Self::incomplete();
        }
        let mut fingerprint = FINGERPRINT_OFFSET;
        for coordinate in locus
            .minimum_leaf
            .into_iter()
            .chain(locus.maximum_leaf_exclusive)
        {
            fingerprint = fingerprint_value(fingerprint, coordinate as u32 as u64);
        }
        fingerprint = fingerprint_value(fingerprint, exact_surface_domain.fingerprint());
        for root in &horizon_roots {
            fingerprint = fingerprint_page_key(fingerprint, *root);
        }
        Self {
            data: Arc::new(PresentationEnvelopeData {
                locus: Some(locus),
                exact_surface_domain,
                horizon_roots,
                exact_radius_bits: exact_radius_metres.to_bits(),
                view_distance_bits: view_distance_metres.to_bits(),
                max_exact_leaves,
                max_horizon_roots,
                complete: true,
                fingerprint,
            }),
        }
    }

    fn incomplete() -> Self {
        Self {
            data: Arc::new(PresentationEnvelopeData {
                locus: None,
                exact_surface_domain: ExactSurfaceDomain::incomplete(),
                horizon_roots: BTreeSet::new(),
                exact_radius_bits: 0,
                view_distance_bits: 0,
                max_exact_leaves: 0,
                max_horizon_roots: 0,
                complete: false,
                fingerprint: 0,
            }),
        }
    }

    pub fn is_complete(&self) -> bool {
        self.data.complete
    }

    pub fn locus(&self) -> Option<PresentationLocus> {
        self.data.locus
    }

    pub fn exact_surface_domain(&self) -> &ExactSurfaceDomain {
        &self.data.exact_surface_domain
    }

    pub fn required_horizon_roots(&self) -> impl Iterator<Item = TerrainPageKey> + '_ {
        self.data.horizon_roots.iter().copied()
    }

    pub fn required_horizon_root_count(&self) -> usize {
        self.data.horizon_roots.len()
    }

    pub fn exact_radius_metres(&self) -> Option<f64> {
        self.data
            .complete
            .then(|| f64::from_bits(self.data.exact_radius_bits))
    }

    pub fn view_distance_metres(&self) -> Option<f64> {
        self.data
            .complete
            .then(|| f64::from_bits(self.data.view_distance_bits))
    }

    pub fn maximum_exact_leaves(&self) -> usize {
        self.data.max_exact_leaves
    }

    pub fn maximum_horizon_roots(&self) -> usize {
        self.data.max_horizon_roots
    }

    fn requires_horizon_owner(&self, key: TerrainPageKey) -> bool {
        key.is_surface()
            && self
                .data
                .horizon_roots
                .iter()
                .any(|root| terrain_page_keys_overlap(key, *root))
    }

    pub fn fingerprint(&self) -> u64 {
        self.data.fingerprint
    }

    pub fn contains_position(&self, position_metres: [f32; 3]) -> bool {
        self.data
            .locus
            .is_some_and(|locus| locus.contains_position(position_metres))
    }

    pub fn clamp_position(&self, position_metres: [f32; 3]) -> Option<[f32; 3]> {
        self.data
            .locus
            .map(|locus| locus.clamp_position(position_metres))
    }

    pub fn shares_storage_with(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.data, &other.data)
    }
}

fn enumerate_locus_dilation_pages(
    locus: PresentationLocus,
    level: u8,
    radius_metres: f64,
    maximum_pages: usize,
) -> Option<BTreeSet<TerrainPageKey>> {
    if level > TERRAIN_COVERAGE_ROOT_LEVEL
        || !radius_metres.is_finite()
        || radius_metres < 0.0
        || maximum_pages == 0
    {
        return None;
    }
    let [locus_minimum, locus_maximum] = locus.horizontal_bounds_metres();
    let page_span_metres = f64::from(TERRAIN_PAGE_EDGE_SAMPLES) * 0.1 * f64::from(1_u32 << level);
    let minimum =
        locus_minimum.map(|coordinate| ((coordinate - radius_metres) / page_span_metres).floor());
    let maximum =
        locus_maximum.map(|coordinate| ((coordinate + radius_metres) / page_span_metres).floor());
    if minimum
        .into_iter()
        .chain(maximum)
        .any(|value| value < f64::from(i32::MIN) || value > f64::from(i32::MAX))
    {
        return None;
    }
    let minimum = minimum.map(|coordinate| coordinate as i32);
    let maximum = maximum.map(|coordinate| coordinate as i32);
    let width = i64::from(maximum[0])
        .checked_sub(i64::from(minimum[0]))?
        .checked_add(1)?;
    let height = i64::from(maximum[1])
        .checked_sub(i64::from(minimum[1]))?
        .checked_add(1)?;
    let candidate_count = usize::try_from(width.checked_mul(height)?).ok()?;
    if candidate_count > maximum_pages {
        return None;
    }
    let radius_squared = radius_metres * radius_metres;
    let conservative_roundoff = f64::EPSILON
        * locus_minimum
            .into_iter()
            .chain(locus_maximum)
            .chain([radius_metres])
            .map(f64::abs)
            .fold(1.0_f64, f64::max)
            .powi(2)
        * 128.0
        + 1.0e-12;
    let mut pages = BTreeSet::new();
    for z in minimum[1]..=maximum[1] {
        for x in minimum[0]..=maximum[0] {
            let key = TerrainPageKey::surface(level, x, z);
            let [page_minimum, page_maximum] = key.horizontal_bounds()?;
            let page_minimum = page_minimum.map(|value| f64::from(value) * 0.1);
            let page_maximum = page_maximum.map(|value| f64::from(value) * 0.1);
            let distance_squared = (0..2)
                .map(|axis| {
                    if page_maximum[axis] < locus_minimum[axis] {
                        (locus_minimum[axis] - page_maximum[axis]).powi(2)
                    } else if locus_maximum[axis] < page_minimum[axis] {
                        (page_minimum[axis] - locus_maximum[axis]).powi(2)
                    } else {
                        0.0
                    }
                })
                .sum::<f64>();
            if distance_squared <= radius_squared + conservative_roundoff {
                pages.insert(key);
            }
        }
    }
    (!pages.is_empty()).then_some(pages)
}

fn fingerprint_value(fingerprint: u64, value: u64) -> u64 {
    (fingerprint ^ value).wrapping_mul(FINGERPRINT_PRIME)
}

fn fingerprint_page_key(mut fingerprint: u64, key: TerrainPageKey) -> u64 {
    fingerprint = fingerprint_value(fingerprint, u64::from(key.level));
    for coordinate in key.coord {
        fingerprint = fingerprint_value(fingerprint, coordinate as u32 as u64);
    }
    fingerprint
}

/// The exact 10 cm surface tiles required by one player-motion prediction.
///
/// This is deliberately a discrete, immutable proof domain rather than a radius interpreted
/// independently by selection, transport, and presentation. Every intersecting level-0 page is
/// enumerated analytically, including pages for which no hierarchy root has arrived yet.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExactSurfaceDomain {
    data: Arc<ExactSurfaceDomainData>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ExactSurfaceDomainInput {
    start_bits: [u64; 3],
    end_bits: [u64; 3],
    radius_bits: u64,
    max_required_leaves: usize,
}

impl ExactSurfaceDomainInput {
    fn new(
        start_metres: [f64; 3],
        end_metres: [f64; 3],
        radius_metres: f64,
        max_required_leaves: usize,
    ) -> Self {
        Self {
            start_bits: start_metres.map(f64::to_bits),
            end_bits: end_metres.map(f64::to_bits),
            radius_bits: radius_metres.to_bits(),
            max_required_leaves,
        }
    }
}

/// Safe immutable reuse for frames whose exact continuous proof input did not change.
///
/// The fingerprint is deliberately absent from the cache key: it is diagnostic evidence, not a
/// collision-free identity. Movement can alter capsule/corner intersections within one leaf, so
/// endpoint page coordinates are likewise insufficient.
#[derive(Clone, Debug, Default)]
pub struct ExactSurfaceDomainCache {
    entry: Option<(ExactSurfaceDomainInput, ExactSurfaceDomain)>,
    rebuilds: u64,
}

impl ExactSurfaceDomainCache {
    pub fn resolve(
        &mut self,
        start_metres: [f64; 3],
        end_metres: [f64; 3],
        radius_metres: f64,
        max_required_leaves: usize,
    ) -> ExactSurfaceDomain {
        let input = ExactSurfaceDomainInput::new(
            start_metres,
            end_metres,
            radius_metres,
            max_required_leaves,
        );
        if let Some((cached_input, cached)) = &self.entry
            && *cached_input == input
        {
            return cached.clone();
        }
        let domain = ExactSurfaceDomain::swept_horizontal_capsule(
            start_metres,
            end_metres,
            radius_metres,
            max_required_leaves,
        );
        self.rebuilds = self.rebuilds.saturating_add(1);
        self.entry = Some((input, domain.clone()));
        domain
    }

    pub const fn rebuilds(&self) -> u64 {
        self.rebuilds
    }
}

#[derive(Debug, Eq, PartialEq)]
struct ExactSurfaceDomainData {
    core_required_leaves: BTreeSet<TerrainPageKey>,
    required_leaves: BTreeSet<TerrainPageKey>,
    ancestors_by_level: Vec<BTreeSet<TerrainPageKey>>,
    core_complete: bool,
    prediction_complete: bool,
    core_fingerprint: u64,
    prediction_fingerprint: u64,
    fingerprint: u64,
}

impl ExactSurfaceDomain {
    /// Builds a conservative exact enumeration of the horizontal swept capsule.
    ///
    /// The candidate page rectangle is bounded before allocation or iteration. Invalid current
    /// input or current-domain overflow produces an incomplete domain which cannot certify
    /// presentation readiness. If only the swept prediction exceeds the bound, the independently
    /// bounded stationary core remains authoritative while prediction diagnostics report debt.
    pub fn swept_horizontal_capsule(
        start_metres: [f64; 3],
        end_metres: [f64; 3],
        radius_metres: f64,
        max_required_leaves: usize,
    ) -> Self {
        let core_required_leaves = enumerate_horizontal_capsule_leaves(
            start_metres,
            start_metres,
            radius_metres,
            max_required_leaves,
        );
        let Some(core_required_leaves) = core_required_leaves else {
            return Self::incomplete();
        };
        let prediction = if start_metres == end_metres {
            Some(core_required_leaves.clone())
        } else {
            enumerate_horizontal_capsule_leaves(
                start_metres,
                end_metres,
                radius_metres,
                max_required_leaves,
            )
        };
        Self::from_parts(core_required_leaves, prediction)
    }

    fn from_parts(
        core_required_leaves: BTreeSet<TerrainPageKey>,
        prediction: Option<BTreeSet<TerrainPageKey>>,
    ) -> Self {
        let core_complete = !core_required_leaves.is_empty();
        let prediction_complete = core_complete && prediction.is_some();
        let required_leaves = prediction.unwrap_or_else(|| core_required_leaves.clone());
        let Some(ancestors_by_level) = required_leaf_ancestors(&required_leaves) else {
            return Self::incomplete();
        };
        let core_fingerprint = required_leaf_fingerprint(&core_required_leaves);
        let prediction_fingerprint = if prediction_complete {
            required_leaf_fingerprint(&required_leaves)
        } else {
            0
        };
        let fingerprint = required_leaf_fingerprint(&required_leaves);
        Self {
            data: Arc::new(ExactSurfaceDomainData {
                core_required_leaves,
                required_leaves,
                ancestors_by_level,
                core_complete,
                prediction_complete,
                core_fingerprint,
                prediction_fingerprint,
                fingerprint,
            }),
        }
    }

    fn incomplete() -> Self {
        Self {
            data: Arc::new(ExactSurfaceDomainData {
                core_required_leaves: BTreeSet::new(),
                required_leaves: BTreeSet::new(),
                ancestors_by_level: vec![
                    BTreeSet::new();
                    usize::from(TERRAIN_COVERAGE_ROOT_LEVEL) + 1
                ],
                core_complete: false,
                prediction_complete: false,
                core_fingerprint: 0,
                prediction_fingerprint: 0,
                fingerprint: 0,
            }),
        }
    }

    pub fn core_is_complete(&self) -> bool {
        self.data.core_complete
    }

    pub fn prediction_is_complete(&self) -> bool {
        self.data.prediction_complete
    }

    /// Whether both the mandatory current core and the optional motion prediction were enumerated.
    pub fn is_complete(&self) -> bool {
        self.data.core_complete && self.data.prediction_complete
    }

    pub fn core_required_leaf_count(&self) -> usize {
        self.data.core_required_leaves.len()
    }

    pub fn prediction_required_leaf_count(&self) -> usize {
        if self.data.prediction_complete {
            self.data.required_leaves.len()
        } else {
            0
        }
    }

    /// Number of leaves which must be exact for this domain's currently provable guarantee.
    ///
    /// This is the full swept prediction when it was bounded successfully, otherwise the
    /// independently bounded current-position core.
    pub fn required_leaf_count(&self) -> usize {
        self.data.required_leaves.len()
    }

    pub fn core_fingerprint(&self) -> u64 {
        self.data.core_fingerprint
    }

    pub fn prediction_fingerprint(&self) -> u64 {
        self.data.prediction_fingerprint
    }

    pub fn fingerprint(&self) -> u64 {
        self.data.fingerprint
    }

    pub fn required_leaves(&self) -> impl Iterator<Item = TerrainPageKey> + '_ {
        self.data.required_leaves.iter().copied()
    }

    pub fn core_required_leaves(&self) -> impl Iterator<Item = TerrainPageKey> + '_ {
        self.data.core_required_leaves.iter().copied()
    }

    pub fn required_pages_at_level(&self, level: u8) -> impl Iterator<Item = TerrainPageKey> + '_ {
        self.data
            .ancestors_by_level
            .get(usize::from(level))
            .into_iter()
            .flatten()
            .copied()
    }

    pub fn intersects_page(&self, key: TerrainPageKey) -> bool {
        self.data.core_complete
            && key.is_surface()
            && self
                .data
                .ancestors_by_level
                .get(usize::from(key.level))
                .is_some_and(|pages| pages.contains(&key))
    }

    /// Clones share immutable enumeration storage. This is exposed for cache invariant tests.
    pub fn shares_storage_with(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.data, &other.data)
    }
}

fn enumerate_horizontal_capsule_leaves(
    start_metres: [f64; 3],
    end_metres: [f64; 3],
    radius_metres: f64,
    max_required_leaves: usize,
) -> Option<BTreeSet<TerrainPageKey>> {
    if !start_metres.into_iter().all(f64::is_finite)
        || !end_metres.into_iter().all(f64::is_finite)
        || !radius_metres.is_finite()
        || radius_metres < 0.0
        || max_required_leaves == 0
    {
        return None;
    }
    let start = [start_metres[0], start_metres[2]];
    let end = [end_metres[0], end_metres[2]];
    let leaf_span_metres = f64::from(TERRAIN_PAGE_EDGE_SAMPLES) * 0.1;
    let minimum = [
        (start[0].min(end[0]) - radius_metres) / leaf_span_metres,
        (start[1].min(end[1]) - radius_metres) / leaf_span_metres,
    ];
    let maximum = [
        (start[0].max(end[0]) + radius_metres) / leaf_span_metres,
        (start[1].max(end[1]) + radius_metres) / leaf_span_metres,
    ];
    let minimum = minimum.map(f64::floor);
    // Page ownership is half-open. `floor` selects the unique owner at the lower capsule bound;
    // at the upper bound it also includes the page owning that boundary point, while the page
    // immediately below is included by the non-zero interior.
    let maximum = maximum.map(f64::floor);
    if minimum
        .into_iter()
        .chain(maximum)
        .any(|value| value < f64::from(i32::MIN) || value > f64::from(i32::MAX))
    {
        return None;
    }
    let minimum = minimum.map(|value| value as i32);
    let maximum = maximum.map(|value| value as i32);
    let width = i64::from(maximum[0])
        .checked_sub(i64::from(minimum[0]))
        .and_then(|value| value.checked_add(1))?;
    let height = i64::from(maximum[1])
        .checked_sub(i64::from(minimum[1]))
        .and_then(|value| value.checked_add(1))?;
    let candidate_count = width
        .checked_mul(height)
        .and_then(|value| usize::try_from(value).ok())?;
    if candidate_count > max_required_leaves {
        return None;
    }

    let radius_squared = radius_metres * radius_metres;
    let coordinate_scale = start
        .into_iter()
        .chain(end)
        .chain([radius_metres])
        .map(f64::abs)
        .fold(1.0_f64, f64::max);
    let conservative_roundoff =
        f64::EPSILON * coordinate_scale * coordinate_scale * 128.0 + 1.0e-12;
    let mut required_leaves = BTreeSet::new();
    for z in minimum[1]..=maximum[1] {
        for x in minimum[0]..=maximum[0] {
            let key = TerrainPageKey::surface(0, x, z);
            let [page_minimum, page_maximum] = key.horizontal_bounds()?;
            let page_minimum = page_minimum.map(|value| f64::from(value) * 0.1);
            let page_maximum = page_maximum.map(|value| f64::from(value) * 0.1);
            if segment_aabb_distance_squared_2d(start, end, page_minimum, page_maximum)
                <= radius_squared + conservative_roundoff
            {
                required_leaves.insert(key);
            }
        }
    }
    if required_leaves.is_empty() {
        return None;
    }
    Some(required_leaves)
}

fn required_leaf_ancestors(
    required_leaves: &BTreeSet<TerrainPageKey>,
) -> Option<Vec<BTreeSet<TerrainPageKey>>> {
    let mut ancestors_by_level =
        vec![BTreeSet::new(); usize::from(TERRAIN_COVERAGE_ROOT_LEVEL) + 1];
    for leaf in required_leaves {
        for level in 0..=TERRAIN_COVERAGE_ROOT_LEVEL {
            let ancestor = leaf.ancestor_at(level)?;
            ancestors_by_level[usize::from(level)].insert(ancestor);
        }
    }
    Some(ancestors_by_level)
}

fn required_leaf_fingerprint(required_leaves: &BTreeSet<TerrainPageKey>) -> u64 {
    if required_leaves.is_empty() {
        return 0;
    }
    let mut fingerprint = FINGERPRINT_OFFSET;
    for leaf in required_leaves {
        fingerprint ^= u64::from(leaf.level);
        fingerprint = fingerprint.wrapping_mul(FINGERPRINT_PRIME);
        for component in leaf.coord {
            fingerprint ^= u64::from(component as u32);
            fingerprint = fingerprint.wrapping_mul(FINGERPRINT_PRIME);
        }
    }
    fingerprint
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VirtualTerrainCapacity {
    pub max_directories: usize,
    pub max_roots: usize,
    pub max_directory_nodes: usize,
    pub max_resident_pages: usize,
    pub max_resident_encoded_bytes: usize,
    pub max_resident_primitives: usize,
    pub max_selected_pages: usize,
    pub max_traversal_nodes: usize,
    pub max_feedback_pages: usize,
}

impl VirtualTerrainCapacity {
    pub const DEVELOPMENT_128_MIB: Self = Self {
        max_directories: 65_536,
        max_roots: 512,
        max_directory_nodes: 299_520,
        max_resident_pages: 8_192,
        max_resident_encoded_bytes: 128 * 1_024 * 1_024,
        max_resident_primitives: 16_777_216,
        max_selected_pages: 16_384,
        max_traversal_nodes: 131_072,
        max_feedback_pages: 256,
    };

    fn validates(self) -> bool {
        self.max_directories > 0
            && self.max_roots > 0
            && self.max_roots <= self.max_directory_nodes
            && self.max_directory_nodes > 0
            && self.max_resident_pages > 0
            && self.max_resident_encoded_bytes > 0
            && self.max_resident_primitives > 0
            && self.max_selected_pages > 0
            && self.max_traversal_nodes >= self.max_selected_pages
            && self.max_feedback_pages > 0
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VirtualTerrainView {
    pub camera_position_metres: [f64; 3],
    pub camera_forward: [f64; 3],
    pub vertical_fov_radians: f64,
    pub aspect_ratio: f64,
    pub viewport_height_pixels: u32,
    pub near_metres: f64,
    pub far_metres: f64,
    pub refine_above_pixels: f64,
    pub coarsen_below_pixels: f64,
    pub wet_specular_sensitivity: f64,
    /// Reference/debug override. Production selection normally follows certified error.
    pub force_exact_leaves: bool,
}

impl VirtualTerrainView {
    /// Preserves exact-domain and ownership refinement while disabling projected-error quality.
    ///
    /// Cold start uses this to publish the first mathematically complete cut before spending
    /// bandwidth on optional visible detail. The ordinary view replaces it immediately after that
    /// first cut commits.
    pub fn ownership_only(mut self) -> Self {
        self.refine_above_pixels = f64::MAX;
        self.coarsen_below_pixels = f64::MAX / 2.0;
        self
    }

    pub fn validates(self) -> bool {
        self.camera_position_metres
            .into_iter()
            .chain(self.camera_forward)
            .all(f64::is_finite)
            && length_squared(self.camera_forward) > f64::EPSILON
            && self.vertical_fov_radians.is_finite()
            && self.vertical_fov_radians > 0.0
            && self.vertical_fov_radians < std::f64::consts::PI
            && self.aspect_ratio.is_finite()
            && self.aspect_ratio > 0.0
            && self.viewport_height_pixels > 0
            && self.near_metres.is_finite()
            && self.near_metres > 0.0
            && self.far_metres.is_finite()
            && self.far_metres > self.near_metres
            && self.refine_above_pixels.is_finite()
            && self.coarsen_below_pixels.is_finite()
            && self.refine_above_pixels > self.coarsen_below_pixels
            && self.coarsen_below_pixels >= 0.0
            && self.wet_specular_sensitivity.is_finite()
            && (0.0..=1.0).contains(&self.wet_specular_sensitivity)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VirtualTerrainCut {
    pub selected_pages: Vec<TerrainPageKey>,
    pub requested_pages: Vec<TerrainPageTransferIdentity>,
    pub refinement_roots: Vec<TerrainPageKey>,
    pub ownerless_roots: Vec<TerrainPageKey>,
    pub fingerprint: u64,
    pub visited_nodes: usize,
    pub selected_primitives: usize,
    pub selected_encoded_bytes: usize,
    pub feedback_overflow: bool,
    pub selection_overflow: bool,
    pub traversal_overflow: bool,
    pub incoherent_replacement_groups: usize,
    pub exact_surface_lod_discontinuities: usize,
}

impl VirtualTerrainCut {
    pub fn is_renderable(&self) -> bool {
        !self.selected_pages.is_empty()
            && self.ownerless_roots.is_empty()
            && !self.selection_overflow
            && !self.traversal_overflow
            && self.exact_surface_lod_discontinuities == 0
    }

    /// Number of explicitly required exact leaves owned by this cut.
    ///
    /// Looking up the required set, rather than scanning selected pages, makes absent roots and
    /// unregistered gaps fail closed.
    pub fn exact_surface_coverage(&self, domain: &ExactSurfaceDomain) -> usize {
        if !domain.core_is_complete()
            || self
                .selected_pages
                .windows(2)
                .any(|pair| pair[0] >= pair[1])
        {
            return 0;
        }
        domain
            .required_leaves()
            .filter(|leaf| self.selected_pages.binary_search(leaf).is_ok())
            .count()
    }

    pub fn exact_surface_core_coverage(&self, domain: &ExactSurfaceDomain) -> usize {
        if !domain.core_is_complete()
            || self
                .selected_pages
                .windows(2)
                .any(|pair| pair[0] >= pair[1])
        {
            return 0;
        }
        domain
            .core_required_leaves()
            .filter(|leaf| self.selected_pages.binary_search(leaf).is_ok())
            .count()
    }

    pub fn preserves_exact_surface_coverage(
        &self,
        committed: &Self,
        domain: &ExactSurfaceDomain,
    ) -> bool {
        domain.core_is_complete()
            && !self
                .selected_pages
                .windows(2)
                .any(|pair| pair[0] >= pair[1])
            && !committed
                .selected_pages
                .windows(2)
                .any(|pair| pair[0] >= pair[1])
            && domain.required_leaves().all(|leaf| {
                committed.selected_pages.binary_search(&leaf).is_err()
                    || self.selected_pages.binary_search(&leaf).is_ok()
            })
    }

    pub fn covers_exact_surface_domain(&self, domain: &ExactSurfaceDomain) -> bool {
        domain.core_is_complete()
            && domain.required_leaf_count() > 0
            && self.exact_surface_coverage(domain) == domain.required_leaf_count()
    }

    /// Whether this cut is the complete, exact owner required by one presentation envelope.
    ///
    /// Horizon coverage is explicit: an unregistered coverage root contributes no selected area
    /// and fails even when every root which happened to reach the local directory is internally
    /// complete. Weighted leaf area avoids recursively expanding a missing level-10 root into
    /// more than a million leaves.
    pub fn covers_presentation_envelope(&self, envelope: &PresentationEnvelope) -> bool {
        if !envelope.is_complete()
            || !self.is_renderable()
            || !self.covers_exact_surface_domain(envelope.exact_surface_domain())
            || self
                .selected_pages
                .windows(2)
                .any(|pair| pair[0] >= pair[1])
        {
            return false;
        }
        let selected = self.selected_pages.iter().copied().collect::<BTreeSet<_>>();
        if selected.iter().any(|key| {
            (key.level.saturating_add(1)..=TERRAIN_COVERAGE_ROOT_LEVEL).any(|level| {
                key.ancestor_at(level)
                    .is_some_and(|ancestor| selected.contains(&ancestor))
            })
        }) {
            return false;
        }
        self.presentation_horizon_coverage(envelope) == envelope.required_horizon_root_count()
    }

    pub fn presentation_horizon_coverage(&self, envelope: &PresentationEnvelope) -> usize {
        if !envelope.is_complete()
            || self
                .selected_pages
                .windows(2)
                .any(|pair| pair[0] >= pair[1])
        {
            return 0;
        }
        let required_leaf_area = 1_u64 << (2 * u32::from(TERRAIN_COVERAGE_ROOT_LEVEL));
        envelope
            .required_horizon_roots()
            .filter(|root| {
                self.selected_pages
                    .iter()
                    .filter(|selected| {
                        selected.is_surface()
                            && selected.ancestor_at(TERRAIN_COVERAGE_ROOT_LEVEL) == Some(*root)
                    })
                    .try_fold(0_u64, |covered, selected| {
                        covered.checked_add(1_u64 << (2 * u32::from(selected.level)))
                    })
                    == Some(required_leaf_area)
            })
            .count()
    }
}

fn segment_aabb_distance_squared_2d(
    start: [f64; 2],
    end: [f64; 2],
    minimum: [f64; 2],
    maximum: [f64; 2],
) -> f64 {
    let direction = [end[0] - start[0], end[1] - start[1]];
    let mut entry = 0.0_f64;
    let mut exit = 1.0_f64;
    for axis in 0..2 {
        if direction[axis].abs() <= f64::EPSILON {
            if start[axis] < minimum[axis] || start[axis] > maximum[axis] {
                exit = -1.0;
                break;
            }
            continue;
        }
        let inverse = 1.0 / direction[axis];
        let first = (minimum[axis] - start[axis]) * inverse;
        let second = (maximum[axis] - start[axis]) * inverse;
        entry = entry.max(first.min(second));
        exit = exit.min(first.max(second));
    }
    if entry <= exit && exit >= 0.0 && entry <= 1.0 {
        return 0.0;
    }

    let point_aabb_distance_squared = |point: [f64; 2]| {
        (0..2)
            .map(|axis| {
                let distance = if point[axis] < minimum[axis] {
                    minimum[axis] - point[axis]
                } else if point[axis] > maximum[axis] {
                    point[axis] - maximum[axis]
                } else {
                    0.0
                };
                distance * distance
            })
            .sum::<f64>()
    };
    let length_squared = direction[0] * direction[0] + direction[1] * direction[1];
    let point_segment_distance_squared = |point: [f64; 2]| {
        if length_squared <= f64::EPSILON {
            return (point[0] - start[0]).powi(2) + (point[1] - start[1]).powi(2);
        }
        let projection = ((point[0] - start[0]) * direction[0]
            + (point[1] - start[1]) * direction[1])
            / length_squared;
        let projection = projection.clamp(0.0, 1.0);
        let nearest = [
            start[0] + direction[0] * projection,
            start[1] + direction[1] * projection,
        ];
        (point[0] - nearest[0]).powi(2) + (point[1] - nearest[1]).powi(2)
    };
    let corners = [
        minimum,
        [minimum[0], maximum[1]],
        [maximum[0], minimum[1]],
        maximum,
    ];
    point_aabb_distance_squared(start)
        .min(point_aabb_distance_squared(end))
        .min(
            corners
                .into_iter()
                .map(point_segment_distance_squared)
                .fold(f64::INFINITY, f64::min),
        )
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum VirtualTerrainError {
    InvalidCapacity,
    InvalidView,
    InvalidDirectory,
    DirectoryCapacity,
    DirectoryCollision(TerrainPageKey),
    UnknownRoot(TerrainPageKey),
    OverlappingRoots(TerrainPageKey, TerrainPageKey),
    IncompleteRootReplacement(TerrainPageKey),
    IncoherentRootReplacement(TerrainPageKey),
    SourceMismatch,
    UnknownPage(TerrainPageKey),
    InvalidPage(TerrainPageKey),
    StalePage(TerrainPageKey),
    ResidentPageCapacity,
    ResidentByteCapacity,
    ResidentPrimitiveCapacity,
}

impl fmt::Display for VirtualTerrainError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidCapacity => formatter.write_str("invalid virtual terrain capacity"),
            Self::InvalidView => formatter.write_str("invalid virtual terrain view"),
            Self::InvalidDirectory => formatter.write_str("invalid virtual terrain directory"),
            Self::DirectoryCapacity => {
                formatter.write_str("virtual terrain directory capacity exceeded")
            }
            Self::DirectoryCollision(key) => {
                write!(formatter, "virtual terrain directory collides at {key:?}")
            }
            Self::UnknownRoot(key) => {
                write!(formatter, "unknown virtual terrain root {key:?}")
            }
            Self::OverlappingRoots(left, right) => write!(
                formatter,
                "virtual terrain roots overlap at {left:?} and {right:?}"
            ),
            Self::IncompleteRootReplacement(key) => {
                write!(
                    formatter,
                    "virtual terrain root replacement is incomplete at {key:?}"
                )
            }
            Self::IncoherentRootReplacement(key) => {
                write!(
                    formatter,
                    "virtual terrain root replacement is incoherent at {key:?}"
                )
            }
            Self::SourceMismatch => formatter.write_str("virtual terrain source mismatch"),
            Self::UnknownPage(key) => write!(formatter, "unknown virtual terrain page {key:?}"),
            Self::InvalidPage(key) => write!(formatter, "invalid virtual terrain page {key:?}"),
            Self::StalePage(key) => write!(formatter, "stale virtual terrain page {key:?}"),
            Self::ResidentPageCapacity => {
                formatter.write_str("virtual terrain resident page capacity exceeded")
            }
            Self::ResidentByteCapacity => {
                formatter.write_str("virtual terrain resident byte capacity exceeded")
            }
            Self::ResidentPrimitiveCapacity => {
                formatter.write_str("virtual terrain resident primitive capacity exceeded")
            }
        }
    }
}

impl std::error::Error for VirtualTerrainError {}

#[derive(Clone, Debug)]
struct ResidentPage {
    page: TerrainPageV1,
    encoded_bytes: usize,
    primitive_count: usize,
    last_selected_frame: u64,
}

#[derive(Debug)]
pub struct VirtualTerrainHierarchy {
    capacity: VirtualTerrainCapacity,
    source_identity_hash: Option<WorldSourceIdentityHash>,
    directory_fingerprints: BTreeSet<[u8; 32]>,
    directory_nodes: BTreeMap<[u8; 32], Vec<TerrainPageKey>>,
    directory_roots: BTreeMap<TerrainPageKey, [u8; 32]>,
    nodes: BTreeMap<TerrainPageKey, TerrainHierarchyNode>,
    active_roots: BTreeSet<TerrainPageKey>,
    resident: BTreeMap<TerrainPageKey, ResidentPage>,
    resident_encoded_bytes: usize,
    resident_primitives: usize,
    coherent_replacements: BTreeSet<TerrainPageKey>,
    refined_last_cut: BTreeSet<TerrainPageKey>,
    balanced_selected_blockers: BTreeMap<TerrainPageKey, BTreeSet<TerrainPageKey>>,
    frame: u64,
}

impl VirtualTerrainHierarchy {
    pub fn new(capacity: VirtualTerrainCapacity) -> Result<Self, VirtualTerrainError> {
        if !capacity.validates() {
            return Err(VirtualTerrainError::InvalidCapacity);
        }
        Ok(Self {
            capacity,
            source_identity_hash: None,
            directory_fingerprints: BTreeSet::new(),
            directory_nodes: BTreeMap::new(),
            directory_roots: BTreeMap::new(),
            nodes: BTreeMap::new(),
            active_roots: BTreeSet::new(),
            resident: BTreeMap::new(),
            resident_encoded_bytes: 0,
            resident_primitives: 0,
            coherent_replacements: BTreeSet::new(),
            refined_last_cut: BTreeSet::new(),
            balanced_selected_blockers: BTreeMap::new(),
            frame: 0,
        })
    }

    pub fn source_identity_hash(&self) -> Option<WorldSourceIdentityHash> {
        self.source_identity_hash
    }

    pub const fn capacity(&self) -> VirtualTerrainCapacity {
        self.capacity
    }

    pub fn directory_node(&self, key: TerrainPageKey) -> Option<TerrainHierarchyNode> {
        self.nodes.get(&key).copied()
    }

    pub fn refined_last_cut(&self) -> impl Iterator<Item = TerrainPageKey> + '_ {
        self.refined_last_cut.iter().copied()
    }

    pub fn selected_fingerprint(&self, selected: &[TerrainPageKey]) -> u64 {
        cut_state_fingerprint(
            cut_fingerprint(selected, self),
            &[],
            false,
            false,
            false,
            0,
            0,
        )
    }

    pub fn nodes(&self) -> impl Iterator<Item = TerrainHierarchyNode> + '_ {
        self.nodes.values().copied()
    }

    pub fn roots(&self) -> impl Iterator<Item = TerrainPageKey> + '_ {
        self.active_roots.iter().copied()
    }

    pub fn registered_roots(&self) -> impl Iterator<Item = TerrainPageKey> + '_ {
        self.directory_roots.keys().copied()
    }

    pub fn replacement_is_resident_and_coherent(&self, key: TerrainPageKey) -> bool {
        self.coherent_replacements.contains(&key)
    }

    fn refresh_replacement_coherence(&mut self, key: TerrainPageKey) {
        self.coherent_replacements.remove(&key);
        let Some(parent) = self.resident.get(&key).map(|resident| &resident.page) else {
            return;
        };
        let Some(children) = key.refinement_children() else {
            return;
        };
        let child_pages = children
            .iter()
            .filter_map(|child| {
                self.resident
                    .get(child)
                    .map(|resident| resident.page.clone())
            })
            .collect::<Vec<_>>();
        if child_pages.len() == children.len()
            && validate_terrain_replacement(parent, &child_pages).is_ok()
        {
            self.coherent_replacements.insert(key);
        }
    }

    pub fn register_region_directory(
        &mut self,
        directory: &TerrainHierarchyDirectoryV1,
    ) -> Result<(), VirtualTerrainError> {
        self.register_directory(directory, true, false)
    }

    /// Registers a validated directory without granting any of its roots render ownership.
    ///
    /// This permits complete replacement roots and their pages to become resident before one
    /// atomic [`Self::set_active_roots`] call transfers ownership to them.
    pub fn register_staging_directory(
        &mut self,
        directory: &TerrainHierarchyDirectoryV1,
    ) -> Result<(), VirtualTerrainError> {
        self.register_directory(directory, false, false)
    }

    /// Extends an existing surface node with one four-child directory segment.
    ///
    /// The existing node remains the authoritative parent predictor while its replacement group
    /// streams. An edit may have already rebuilt the independently generated segment root; that
    /// repeated root is metadata, not a second owner, so its newer identity does not replace the
    /// resident parent. The complete children still have to pass the ordinary boundary-coherence
    /// proof before the cut can refine.
    pub fn register_refinement_directory(
        &mut self,
        directory: &TerrainHierarchyDirectoryV1,
    ) -> Result<(), VirtualTerrainError> {
        let roots = directory.roots().collect::<Vec<_>>();
        let Some(root) = roots
            .first()
            .filter(|root| roots.len() == 1 && root.key.is_surface())
        else {
            return Err(VirtualTerrainError::InvalidDirectory);
        };
        let expected = root
            .key
            .refinement_children()
            .map(|children| {
                children
                    .into_iter()
                    .chain([root.key])
                    .collect::<BTreeSet<_>>()
            })
            .ok_or(VirtualTerrainError::InvalidDirectory)?;
        if directory
            .nodes
            .iter()
            .map(|node| node.key)
            .collect::<BTreeSet<_>>()
            != expected
            || !self.nodes.contains_key(&root.key)
        {
            return Err(VirtualTerrainError::InvalidDirectory);
        }
        self.register_directory(directory, false, true)
    }

    fn register_directory(
        &mut self,
        directory: &TerrainHierarchyDirectoryV1,
        activate_roots: bool,
        refinement: bool,
    ) -> Result<(), VirtualTerrainError> {
        if !directory.validates_identity() {
            return Err(VirtualTerrainError::InvalidDirectory);
        }
        if self
            .source_identity_hash
            .is_some_and(|source| source != directory.source_identity_hash)
        {
            return Err(VirtualTerrainError::SourceMismatch);
        }
        if self
            .directory_fingerprints
            .contains(&directory.content_fingerprint)
        {
            return Ok(());
        }
        let new_node_count = directory
            .nodes
            .iter()
            .filter(|node| !self.nodes.contains_key(&node.key))
            .count();
        if self.directory_fingerprints.len() >= self.capacity.max_directories
            || self.nodes.len().saturating_add(new_node_count) > self.capacity.max_directory_nodes
            || (!refinement
                && self.directory_roots.len().saturating_add(
                    directory
                        .roots()
                        .filter(|node| !self.directory_roots.contains_key(&node.key))
                        .count(),
                ) > self.capacity.max_roots)
        {
            return Err(VirtualTerrainError::DirectoryCapacity);
        }
        for node in &directory.nodes {
            if let Some(existing) = self.nodes.get(&node.key) {
                let collision = if refinement {
                    if node.is_root {
                        continue;
                    }
                    let mut normalized = *node;
                    normalized.has_children = existing.has_children;
                    normalized.is_root = existing.is_root;
                    normalized != *existing
                } else {
                    node != existing
                };
                if collision {
                    return Err(VirtualTerrainError::DirectoryCollision(node.key));
                }
            }
        }
        self.source_identity_hash = Some(directory.source_identity_hash);
        self.directory_fingerprints
            .insert(directory.content_fingerprint);
        self.directory_nodes.insert(
            directory.content_fingerprint,
            directory.nodes.iter().map(|node| node.key).collect(),
        );
        for node in &directory.nodes {
            if let Some(existing) = self.nodes.get_mut(&node.key) {
                existing.has_children |= refinement && node.has_children;
            } else {
                let mut inserted = *node;
                if refinement {
                    inserted.is_root = false;
                }
                self.nodes.insert(node.key, inserted);
            }
            if node.is_root && !refinement {
                if activate_roots {
                    self.active_roots.insert(node.key);
                }
                self.directory_roots
                    .insert(node.key, directory.content_fingerprint);
            }
        }
        Ok(())
    }

    /// Atomically selects the non-overlapping directory roots that own the next global cut.
    ///
    /// Registered but inactive roots remain resident staging data. This is the seam-free
    /// replacement primitive for progressively swapping a coarse regional root for its complete
    /// child group without ever drawing both ownership levels.
    pub fn set_active_roots(
        &mut self,
        roots: impl IntoIterator<Item = TerrainPageKey>,
    ) -> Result<(), VirtualTerrainError> {
        let roots = roots.into_iter().collect::<BTreeSet<_>>();
        if roots.len() > self.capacity.max_roots {
            return Err(VirtualTerrainError::DirectoryCapacity);
        }
        if let Some(unknown) = roots.iter().find(|root| !self.nodes.contains_key(root)) {
            return Err(VirtualTerrainError::UnknownRoot(*unknown));
        }
        for (index, left) in roots.iter().enumerate() {
            if let Some(right) = roots
                .iter()
                .skip(index + 1)
                .find(|right| terrain_page_keys_overlap(*left, **right))
            {
                return Err(VirtualTerrainError::OverlappingRoots(*left, *right));
            }
        }
        self.validate_root_transition(&roots)?;
        self.active_roots = roots;
        self.refined_last_cut.retain(|key| {
            self.active_roots
                .iter()
                .any(|root| key.ancestor_at(root.level) == Some(*root))
        });
        let active_roots = self.active_roots.clone();
        self.balanced_selected_blockers.retain(|owner, blockers| {
            let owner_is_active = active_roots
                .iter()
                .any(|root| owner.ancestor_at(root.level) == Some(*root));
            blockers.retain(|blocker| {
                active_roots
                    .iter()
                    .any(|root| blocker.ancestor_at(root.level) == Some(*root))
            });
            owner_is_active && !blockers.is_empty()
        });
        Ok(())
    }

    fn validate_root_transition(
        &self,
        next_roots: &BTreeSet<TerrainPageKey>,
    ) -> Result<(), VirtualTerrainError> {
        for parent in &self.active_roots {
            let replacements = next_roots
                .iter()
                .filter(|candidate| {
                    candidate.level < parent.level
                        && candidate.ancestor_at(parent.level) == Some(*parent)
                })
                .copied()
                .collect::<BTreeSet<_>>();
            if replacements.is_empty() {
                continue;
            }
            let expected = parent
                .refinement_children()
                .map(|children| children.into_iter().collect())
                .ok_or(VirtualTerrainError::IncompleteRootReplacement(*parent))?;
            if replacements != expected {
                return Err(VirtualTerrainError::IncompleteRootReplacement(*parent));
            }
            if !self.replacement_is_resident_and_coherent(*parent) {
                return Err(VirtualTerrainError::IncoherentRootReplacement(*parent));
            }
        }
        for parent in next_roots {
            let replacements = self
                .active_roots
                .iter()
                .filter(|candidate| {
                    candidate.level < parent.level
                        && candidate.ancestor_at(parent.level) == Some(*parent)
                })
                .copied()
                .collect::<BTreeSet<_>>();
            if replacements.is_empty() {
                continue;
            }
            let expected = parent
                .refinement_children()
                .map(|children| children.into_iter().collect())
                .ok_or(VirtualTerrainError::IncompleteRootReplacement(*parent))?;
            if replacements != expected {
                return Err(VirtualTerrainError::IncompleteRootReplacement(*parent));
            }
            if !self.replacement_is_resident_and_coherent(*parent) {
                return Err(VirtualTerrainError::IncoherentRootReplacement(*parent));
            }
        }
        Ok(())
    }

    pub fn install_page(&mut self, page: TerrainPageV1) -> Result<(), VirtualTerrainError> {
        let Some(node) = self.nodes.get(&page.key) else {
            return Err(VirtualTerrainError::UnknownPage(page.key));
        };
        if page.source_identity_hash
            != self
                .source_identity_hash
                .unwrap_or(page.source_identity_hash)
        {
            return Err(VirtualTerrainError::SourceMismatch);
        }
        if !page.validates_identity()
            || page.revision != node.revision
            || page.content_fingerprint != node.content_fingerprint
            || page.errors != node.errors
            || page.topology != node.topology
            || page.representation.kind() != node.representation
        {
            return Err(VirtualTerrainError::InvalidPage(page.key));
        }
        if let Some(existing) = self.resident.get(&page.key) {
            return if existing.page.content_fingerprint == page.content_fingerprint
                && existing.page.revision == page.revision
            {
                Ok(())
            } else {
                Err(VirtualTerrainError::StalePage(page.key))
            };
        }
        let encoded_bytes = encode_terrain_page(&page)
            .map_err(|_| VirtualTerrainError::InvalidPage(page.key))?
            .len();
        if encoded_bytes != node.encoded_bytes as usize {
            return Err(VirtualTerrainError::InvalidPage(page.key));
        }
        let primitive_count = page_primitive_count(&page);
        if self.resident.len() >= self.capacity.max_resident_pages {
            return Err(VirtualTerrainError::ResidentPageCapacity);
        }
        if self.resident_encoded_bytes.saturating_add(encoded_bytes)
            > self.capacity.max_resident_encoded_bytes
        {
            return Err(VirtualTerrainError::ResidentByteCapacity);
        }
        if self.resident_primitives.saturating_add(primitive_count)
            > self.capacity.max_resident_primitives
        {
            return Err(VirtualTerrainError::ResidentPrimitiveCapacity);
        }
        self.resident_encoded_bytes += encoded_bytes;
        self.resident_primitives += primitive_count;
        let key = page.key;
        self.resident.insert(
            key,
            ResidentPage {
                page,
                encoded_bytes,
                primitive_count,
                last_selected_frame: 0,
            },
        );
        self.refresh_replacement_coherence(key);
        if let Some(parent) = key.parent() {
            self.refresh_replacement_coherence(parent);
        }
        Ok(())
    }

    pub fn resident_page(&self, key: TerrainPageKey) -> Option<&TerrainPageV1> {
        self.resident.get(&key).map(|resident| &resident.page)
    }

    pub fn resident_page_keys(&self) -> impl Iterator<Item = TerrainPageKey> + '_ {
        self.resident.keys().copied()
    }

    pub fn resident_usage(&self) -> (usize, usize, usize) {
        (
            self.resident.len(),
            self.resident_encoded_bytes,
            self.resident_primitives,
        )
    }

    pub fn remove_page(&mut self, key: TerrainPageKey) -> bool {
        let Some(resident) = self.resident.remove(&key) else {
            return false;
        };
        self.resident_encoded_bytes = self
            .resident_encoded_bytes
            .saturating_sub(resident.encoded_bytes);
        self.resident_primitives = self
            .resident_primitives
            .saturating_sub(resident.primitive_count);
        self.coherent_replacements.remove(&key);
        self.refined_last_cut.remove(&key);
        self.balanced_selected_blockers.remove(&key);
        if let Some(parent) = key.parent() {
            self.coherent_replacements.remove(&parent);
            self.refined_last_cut.remove(&parent);
        }
        true
    }

    /// Removes the complete directory containing `root` and every resident page it described.
    ///
    /// Fixed production regions are disjoint, but the directory format permits multiple roots.
    /// They are therefore retired as one immutable directory unit.
    pub fn remove_region_directory(&mut self, root: TerrainPageKey) -> Vec<TerrainPageKey> {
        let Some(fingerprint) = self.directory_roots.get(&root).copied() else {
            return Vec::new();
        };
        let related = self
            .directory_nodes
            .iter()
            .filter(|(candidate, keys)| {
                **candidate == fingerprint
                    || keys
                        .iter()
                        .all(|key| key.ancestor_at(root.level) == Some(root))
            })
            .map(|(fingerprint, _)| *fingerprint)
            .collect::<BTreeSet<_>>();
        let key_set = related
            .iter()
            .filter_map(|fingerprint| self.directory_nodes.remove(fingerprint))
            .flatten()
            .collect::<BTreeSet<_>>();
        for fingerprint in &related {
            self.directory_fingerprints.remove(fingerprint);
        }
        self.directory_roots
            .retain(|_, owner| !related.contains(owner));
        self.active_roots.retain(|key| !key_set.contains(key));
        for key in &key_set {
            self.remove_page(*key);
            self.nodes.remove(key);
            self.refined_last_cut.remove(key);
            self.balanced_selected_blockers.remove(key);
        }
        self.balanced_selected_blockers.retain(|owner, blockers| {
            blockers.retain(|blocker| self.nodes.contains_key(blocker));
            self.nodes.contains_key(owner) && !blockers.is_empty()
        });
        if self.directory_fingerprints.is_empty() {
            self.source_identity_hash = None;
        }
        key_set.into_iter().collect()
    }

    pub fn select_cut(
        &mut self,
        view: VirtualTerrainView,
        exact_surface_domain: &ExactSurfaceDomain,
    ) -> Result<VirtualTerrainCut, VirtualTerrainError> {
        self.select_cut_internal(view, exact_surface_domain, None)
    }

    /// Selects a cut whose traversal is complete for a frozen presentation envelope.
    ///
    /// Ordinary view selection may ignore region roots behind the camera. Publication selection
    /// may not: every active root inside the envelope horizon is a mandatory owner even when it is
    /// currently outside the frustum, because the player can turn anywhere inside the committed
    /// locus without waiting for a new terrain transaction.
    pub fn select_presentation_cut(
        &mut self,
        view: VirtualTerrainView,
        envelope: &PresentationEnvelope,
    ) -> Result<VirtualTerrainCut, VirtualTerrainError> {
        self.select_cut_internal(view, envelope.exact_surface_domain(), Some(envelope))
    }

    fn select_cut_internal(
        &mut self,
        view: VirtualTerrainView,
        exact_surface_domain: &ExactSurfaceDomain,
        presentation_envelope: Option<&PresentationEnvelope>,
    ) -> Result<VirtualTerrainCut, VirtualTerrainError> {
        if !view.validates() {
            return Err(VirtualTerrainError::InvalidView);
        }
        self.frame = self.frame.wrapping_add(1).max(1);
        let frame = self.frame;
        let prior_refined = self.refined_last_cut.clone();
        let prior_balanced_selected_blockers = self.balanced_selected_blockers.clone();
        let mut builder = CutBuilder {
            hierarchy: self,
            view,
            exact_surface_domain,
            presentation_envelope,
            frame,
            prior_refined: &prior_refined,
            prior_balanced_selected_blockers,
            next_refined: BTreeSet::new(),
            next_balanced_refined: BTreeSet::new(),
            next_balanced_selected: BTreeSet::new(),
            next_balanced_selected_blockers: BTreeMap::new(),
            selected: Vec::new(),
            selected_owners: BTreeMap::new(),
            visited_active_roots: BTreeSet::new(),
            requests: BTreeSet::new(),
            refinement_requests: BTreeSet::new(),
            ownerless_roots: Vec::new(),
            visited_nodes: 0,
            selected_primitives: 0,
            selected_encoded_bytes: 0,
            feedback_overflow: false,
            selection_overflow: false,
            traversal_overflow: false,
            incoherent_replacement_groups: 0,
        };
        let roots = builder
            .hierarchy
            .active_roots
            .iter()
            .copied()
            .filter(|key| {
                builder.hierarchy.nodes.get(key).is_some_and(|node| {
                    page_is_visible(node.bounds, view)
                        || exact_surface_domain.intersects_page(*key)
                        || presentation_envelope
                            .is_some_and(|envelope| envelope.requires_horizon_owner(*key))
                })
            })
            .collect::<Vec<_>>();
        builder.visited_active_roots.extend(roots.iter().copied());
        for root in roots {
            if builder.selected.len() >= builder.hierarchy.capacity.max_selected_pages {
                builder.selection_overflow = true;
                builder.ownerless_roots.push(root);
                builder.request(root);
                continue;
            }
            builder.visit(root, true, root);
        }
        builder.balance_surface_lod();
        if let Some(envelope) = presentation_envelope {
            for root in envelope.required_horizon_roots() {
                if !builder.fully_covers_horizon_root(root) {
                    builder.ownerless_roots.push(root);
                }
            }
        }
        if builder.ownerless_roots.is_empty() && !builder.selection_is_exact_active_root_partition()
        {
            builder.traversal_overflow = true;
        }
        builder.selected.sort_unstable();
        builder.ownerless_roots.sort_unstable();
        builder.ownerless_roots.dedup();
        let exact_surface_lod_discontinuities =
            exact_surface_lod_discontinuity_edges(&builder.selected);
        let mut requested_pages = builder.requests.into_iter().collect::<Vec<_>>();
        requested_pages.sort_unstable_by_key(|identity| identity.key);
        let refinement_roots = builder.refinement_requests.into_iter().collect();
        let renderable = !builder.selected.is_empty()
            && builder.ownerless_roots.is_empty()
            && !builder.selection_overflow
            && !builder.traversal_overflow
            && exact_surface_lod_discontinuities == 0;
        let fingerprint = cut_state_fingerprint(
            cut_fingerprint(&builder.selected, builder.hierarchy),
            &builder.ownerless_roots,
            builder.feedback_overflow,
            builder.selection_overflow,
            builder.traversal_overflow,
            builder.incoherent_replacement_groups,
            exact_surface_lod_discontinuities,
        );
        if renderable {
            builder.hierarchy.refined_last_cut = builder.next_refined.clone();
            builder.hierarchy.balanced_selected_blockers =
                builder.next_balanced_selected_blockers.clone();
        }
        Ok(VirtualTerrainCut {
            selected_pages: builder.selected,
            requested_pages,
            refinement_roots,
            ownerless_roots: builder.ownerless_roots,
            fingerprint,
            visited_nodes: builder.visited_nodes,
            selected_primitives: builder.selected_primitives,
            selected_encoded_bytes: builder.selected_encoded_bytes,
            feedback_overflow: builder.feedback_overflow,
            selection_overflow: builder.selection_overflow,
            traversal_overflow: builder.traversal_overflow,
            incoherent_replacement_groups: builder.incoherent_replacement_groups,
            exact_surface_lod_discontinuities,
        })
    }
}

/// Finds fine edge segments whose selected neighbor skips an intermediate surface level.
///
/// Looking outward from the finer page makes the audit bounded by four directions times the
/// hierarchy depth rather than comparing every selected page with every other page.
fn surface_lod_discontinuity_pairs(
    selected: &[TerrainPageKey],
) -> Vec<(TerrainPageKey, TerrainPageKey)> {
    let selected = selected.iter().copied().collect::<BTreeSet<_>>();
    let maximum_level = selected
        .iter()
        .filter(|key| key.is_surface())
        .map(|key| key.level)
        .max()
        .unwrap_or(0);
    let mut discontinuities = Vec::new();
    for key in selected.iter().copied().filter(|key| key.is_surface()) {
        for neighbor in [
            [key.coord[0].saturating_sub(1), key.coord[2]],
            [key.coord[0].saturating_add(1), key.coord[2]],
            [key.coord[0], key.coord[2].saturating_sub(1)],
            [key.coord[0], key.coord[2].saturating_add(1)],
        ] {
            let same_level = TerrainPageKey::surface(key.level, neighbor[0], neighbor[1]);
            if let Some(coarse) =
                ((key.level.saturating_add(2))..=maximum_level).find_map(|level| {
                    same_level
                        .ancestor_at(level)
                        .filter(|ancestor| selected.contains(ancestor))
                })
            {
                discontinuities.push((key, coarse));
            }
        }
    }
    discontinuities
}

fn exact_surface_lod_discontinuity_edges(selected: &[TerrainPageKey]) -> usize {
    surface_lod_discontinuity_pairs(selected).len()
}

fn has_ancestor_in(mut key: TerrainPageKey, ancestors: &BTreeSet<TerrainPageKey>) -> bool {
    loop {
        if ancestors.contains(&key) {
            return true;
        }
        let Some(parent) = key.parent() else {
            return false;
        };
        key = parent;
    }
}

/// Partitions skipped-level edges into independently executable transition frontiers.
///
/// Edges are connected when they share a selected page or when the quadtree domains changed by
/// their nominal refine/coarsen operations overlap. The latter joins distinct fine edge segments
/// that would otherwise try to replace the same ancestor independently.
fn surface_lod_discontinuity_components(
    discontinuities: &[(TerrainPageKey, TerrainPageKey)],
) -> Vec<Vec<(TerrainPageKey, TerrainPageKey)>> {
    fn root(parents: &mut [usize], mut index: usize) -> usize {
        while parents[index] != index {
            parents[index] = parents[parents[index]];
            index = parents[index];
        }
        index
    }

    fn join(parents: &mut [usize], left: usize, right: usize) {
        let left = root(parents, left);
        let right = root(parents, right);
        let (minimum, maximum) = if left < right {
            (left, right)
        } else {
            (right, left)
        };
        parents[maximum] = minimum;
    }

    let domains = |(fine, coarse): &(TerrainPageKey, TerrainPageKey)| {
        [
            coarse
                .level
                .checked_sub(1)
                .and_then(|level| fine.ancestor_at(level))
                .unwrap_or(*fine),
            *coarse,
        ]
    };
    let mut edges = discontinuities.to_vec();
    edges.sort_unstable();
    edges.dedup();
    let mut parents = (0..edges.len()).collect::<Vec<_>>();
    let mut exact_domains = BTreeMap::new();
    for (index, edge) in edges.iter().enumerate() {
        for domain in domains(edge) {
            exact_domains.entry(domain).or_insert(index);
        }
    }
    // Quadtree domains overlap iff they are equal or one is an ancestor of the other. Looking up
    // every real ancestor in the prebuilt exact-domain index therefore finds all conflicts in
    // O(edges * hierarchy depth), without the quadratic all-pairs scan that a large frontier
    // would otherwise impose.
    for (index, edge) in edges.iter().enumerate() {
        for domain in domains(edge) {
            let mut ancestor = Some(domain);
            while let Some(key) = ancestor {
                if let Some(other) = exact_domains.get(&key) {
                    join(&mut parents, index, *other);
                }
                ancestor = key.parent();
            }
        }
    }
    let mut grouped = BTreeMap::<usize, Vec<_>>::new();
    for (index, edge) in edges.into_iter().enumerate() {
        grouped
            .entry(root(&mut parents, index))
            .or_default()
            .push(edge);
    }
    let mut components = grouped.into_values().collect::<Vec<_>>();
    for component in &mut components {
        component.sort_unstable();
    }
    components.sort_unstable_by_key(|component| component[0]);
    components
}

fn terrain_page_keys_overlap(left: TerrainPageKey, right: TerrainPageKey) -> bool {
    if left.level == right.level {
        return left == right;
    }
    if left.level > right.level {
        right.ancestor_at(left.level) == Some(left)
    } else {
        left.ancestor_at(right.level) == Some(right)
    }
}

struct CutBuilder<'a> {
    hierarchy: &'a mut VirtualTerrainHierarchy,
    view: VirtualTerrainView,
    exact_surface_domain: &'a ExactSurfaceDomain,
    presentation_envelope: Option<&'a PresentationEnvelope>,
    frame: u64,
    prior_refined: &'a BTreeSet<TerrainPageKey>,
    prior_balanced_selected_blockers: BTreeMap<TerrainPageKey, BTreeSet<TerrainPageKey>>,
    next_refined: BTreeSet<TerrainPageKey>,
    next_balanced_refined: BTreeSet<TerrainPageKey>,
    next_balanced_selected: BTreeSet<TerrainPageKey>,
    next_balanced_selected_blockers: BTreeMap<TerrainPageKey, BTreeSet<TerrainPageKey>>,
    selected: Vec<TerrainPageKey>,
    selected_owners: BTreeMap<TerrainPageKey, TerrainPageKey>,
    visited_active_roots: BTreeSet<TerrainPageKey>,
    requests: BTreeSet<TerrainPageTransferIdentity>,
    refinement_requests: BTreeSet<TerrainPageKey>,
    ownerless_roots: Vec<TerrainPageKey>,
    visited_nodes: usize,
    selected_primitives: usize,
    selected_encoded_bytes: usize,
    feedback_overflow: bool,
    selection_overflow: bool,
    traversal_overflow: bool,
    incoherent_replacement_groups: usize,
}

impl CutBuilder<'_> {
    fn fully_covers_horizon_root(&self, root: TerrainPageKey) -> bool {
        let required_leaf_area = 1_u128 << (2 * u32::from(TERRAIN_COVERAGE_ROOT_LEVEL));
        self.selected
            .iter()
            .filter(|selected| {
                selected.is_surface()
                    && selected.ancestor_at(TERRAIN_COVERAGE_ROOT_LEVEL) == Some(root)
            })
            .try_fold(0_u128, |covered, selected| {
                covered.checked_add(1_u128 << (2 * u32::from(selected.level)))
            })
            == Some(required_leaf_area)
    }

    /// Makes every surface edge differ by at most one level without adding seam geometry.
    ///
    /// Replacing a coarse page with its complete coherent child group preserves the complete
    /// half-open partition and preserves exact 10 cm ownership near the player. If those children
    /// have not streamed yet, replacing the finer subtree with its resident ancestor is the only
    /// mathematically conforming cut available without inventing patch geometry. The forced
    /// selection is recorded so GPU traversal reproduces the same temporary cut exactly.
    fn balance_surface_lod(&mut self) {
        let mut passes = 0_usize;
        let maximum_passes = self
            .selected
            .iter()
            .filter(|key| key.is_surface())
            .map(|key| usize::from(key.level))
            .max()
            .unwrap_or(0)
            .saturating_add(1)
            .saturating_mul(2);
        loop {
            let discontinuities = surface_lod_discontinuity_pairs(&self.selected);
            if discontinuities.is_empty() {
                break;
            }
            if passes >= maximum_passes {
                self.traversal_overflow = true;
                break;
            }
            passes = passes.saturating_add(1);

            let mut components = surface_lod_discontinuity_components(&discontinuities);
            components.sort_by(|left, right| {
                let left_exact = self.surface_component_requires_exact(left);
                let right_exact = self.surface_component_requires_exact(right);
                right_exact
                    .cmp(&left_exact)
                    .then_with(|| {
                        if left_exact {
                            self.surface_component_distance(left)
                                .total_cmp(&self.surface_component_distance(right))
                        } else {
                            std::cmp::Ordering::Equal
                        }
                    })
                    .then_with(|| {
                        self.surface_component_refinement_cost(left)
                            .cmp(&self.surface_component_refinement_cost(right))
                    })
                    .then_with(|| left[0].cmp(&right[0]))
            });

            let mut refined_frontier = false;
            let mut unresolved = Vec::new();
            let mut capacity_blocked_exact = false;
            for component in components {
                let coarse_pages = component
                    .iter()
                    .map(|(_, coarse)| *coarse)
                    .collect::<BTreeSet<_>>();
                let replacements = coarse_pages
                    .iter()
                    .filter_map(|coarse| {
                        coarse
                            .refinement_children()
                            .map(|children| (*coarse, children))
                    })
                    .collect::<Vec<_>>();
                let coherent = replacements.len() == coarse_pages.len()
                    && replacements.iter().all(|(coarse, _)| {
                        !self.next_balanced_selected.contains(coarse)
                            && self.hierarchy.replacement_is_resident_and_coherent(*coarse)
                    });
                if !coherent {
                    for (coarse, children) in &replacements {
                        if !self.next_balanced_selected.contains(coarse)
                            && !self.hierarchy.replacement_is_resident_and_coherent(*coarse)
                        {
                            self.record_unavailable_replacement(*coarse, children);
                        }
                    }
                    unresolved.push(component);
                    continue;
                }
                let additional_pages = replacements
                    .iter()
                    .map(|(_, children)| children.len().saturating_sub(1))
                    .sum::<usize>();
                let exact = self.surface_component_requires_exact(&component);
                if (!exact && capacity_blocked_exact)
                    || self.selected.len().saturating_add(additional_pages)
                        > self.hierarchy.capacity.max_selected_pages
                {
                    if exact {
                        capacity_blocked_exact = true;
                    }
                    unresolved.push(component);
                    continue;
                }
                for (coarse, children) in replacements {
                    if self.refine_surface_frontier(coarse, children) {
                        refined_frontier = true;
                    }
                }
            }
            if refined_frontier {
                // Refinement exposes a different edge set. Rebuild the frontier graph before
                // deciding which still-unresolved components need a temporary coarser owner.
                continue;
            }

            let targets = self.surface_coarsening_targets(unresolved);
            let coarsened_frontier = self.coarsen_surface_frontiers(targets);
            if !coarsened_frontier {
                if capacity_blocked_exact {
                    self.selection_overflow = true;
                }
                // Ownership or exact-player constraints can make a discontinuity intentionally
                // unresolved. The cut remains a valid partition but is not publishable until its
                // requested coarse-side replacement arrives.
                break;
            }
        }
        self.selected.sort_unstable();
        self.selected.dedup();
        if !self
            .next_balanced_refined
            .is_disjoint(&self.next_balanced_selected)
        {
            self.traversal_overflow = true;
        }
        self.selected_primitives = 0;
        self.selected_encoded_bytes = 0;
        for key in &self.selected {
            let Some(resident) = self.hierarchy.resident.get_mut(key) else {
                continue;
            };
            resident.last_selected_frame = self.frame;
            self.selected_primitives = self
                .selected_primitives
                .saturating_add(resident.primitive_count);
            self.selected_encoded_bytes = self
                .selected_encoded_bytes
                .saturating_add(resident.encoded_bytes);
        }
    }

    fn record_unavailable_replacement(
        &mut self,
        coarse: TerrainPageKey,
        children: &[TerrainPageKey],
    ) {
        let node_has_children = self
            .hierarchy
            .nodes
            .get(&coarse)
            .is_some_and(|node| node.has_children);
        if !node_has_children && coarse.level > 0 {
            if self.refinement_requests.len() < self.hierarchy.capacity.max_feedback_pages {
                self.refinement_requests.insert(coarse);
            } else {
                self.feedback_overflow = true;
            }
        } else if children
            .iter()
            .all(|child| self.hierarchy.resident.contains_key(child))
        {
            self.incoherent_replacement_groups =
                self.incoherent_replacement_groups.saturating_add(1);
        } else {
            for child in children {
                if !self.hierarchy.resident.contains_key(child) {
                    self.request(*child);
                }
            }
        }
    }

    fn refine_surface_frontier(
        &mut self,
        coarse: TerrainPageKey,
        children: Vec<TerrainPageKey>,
    ) -> bool {
        if self.next_balanced_selected.contains(&coarse) {
            self.traversal_overflow = true;
            return false;
        }
        let Some(owner) = self.selected_owners.remove(&coarse) else {
            self.traversal_overflow = true;
            return false;
        };
        self.selected.retain(|key| *key != coarse);
        for child in children {
            self.selected.push(child);
            self.selected_owners.insert(child, owner);
        }
        self.next_refined.insert(coarse);
        self.next_balanced_refined.insert(coarse);
        true
    }

    fn surface_page_requires_exact(&self, key: TerrainPageKey) -> bool {
        if !key.is_surface() {
            return false;
        }
        self.view.force_exact_leaves || self.exact_surface_domain.intersects_page(key)
    }

    fn surface_component_requires_exact(
        &self,
        component: &[(TerrainPageKey, TerrainPageKey)],
    ) -> bool {
        component.iter().any(|(fine, coarse)| {
            self.surface_page_requires_exact(*fine) || self.surface_page_requires_exact(*coarse)
        })
    }

    fn surface_component_distance(&self, component: &[(TerrainPageKey, TerrainPageKey)]) -> f64 {
        component
            .iter()
            .flat_map(|(fine, coarse)| [*fine, *coarse])
            .filter_map(|key| self.hierarchy.nodes.get(&key))
            .map(|node| distance_to_page_metres(node.bounds, self.view.camera_position_metres))
            .min_by(f64::total_cmp)
            .unwrap_or(f64::INFINITY)
    }

    fn surface_component_refinement_cost(
        &self,
        component: &[(TerrainPageKey, TerrainPageKey)],
    ) -> usize {
        component
            .iter()
            .map(|(_, coarse)| *coarse)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .map(|coarse| {
                coarse
                    .refinement_children()
                    .map_or(TERRAIN_PAGE_MAX_CHILDREN, |children| children.len())
                    .saturating_sub(1)
            })
            .sum()
    }

    fn surface_coarsening_target(
        &self,
        fine: TerrainPageKey,
        coarse: TerrainPageKey,
    ) -> Option<(TerrainPageKey, TerrainPageKey)> {
        let owner = *self.selected_owners.get(&fine)?;
        if !owner.is_surface()
            || fine.ancestor_at(owner.level) != Some(owner)
            || owner.level < fine.level
        {
            return None;
        }
        let desired_level = coarse.level.checked_sub(1)?.min(owner.level);
        let mut target = fine.ancestor_at(desired_level)?;
        while !self.hierarchy.resident.contains_key(&target) {
            let parent = target.parent()?;
            if parent.level > owner.level || parent.ancestor_at(owner.level) != Some(owner) {
                return None;
            }
            target = parent;
        }
        Some((target, owner))
    }

    fn surface_coarsening_targets(
        &mut self,
        unresolved: Vec<Vec<(TerrainPageKey, TerrainPageKey)>>,
    ) -> BTreeMap<TerrainPageKey, (TerrainPageKey, BTreeSet<TerrainPageKey>)> {
        let mut targets =
            BTreeMap::<TerrainPageKey, (TerrainPageKey, BTreeSet<TerrainPageKey>)>::new();
        for component in unresolved {
            for (fine, coarse) in component {
                if self.surface_page_requires_exact(fine) {
                    continue;
                }
                let Some((target, owner)) = self.surface_coarsening_target(fine, coarse) else {
                    continue;
                };
                let entry = targets
                    .entry(target)
                    .or_insert_with(|| (owner, BTreeSet::new()));
                if entry.0 != owner {
                    self.traversal_overflow = true;
                }
                // Persist only blockers on this target's causal edge. Copying the union for the
                // whole connected component into every temporary owner makes a long seam ring
                // retain and recheck a quadratic blocker matrix.
                if let Some(blockers) = self.next_balanced_selected_blockers.get(&coarse) {
                    entry.1.extend(blockers.iter().copied());
                } else if !self.hierarchy.replacement_is_resident_and_coherent(coarse) {
                    entry.1.insert(coarse);
                }
            }
        }
        targets
    }

    /// Coarsens every disjoint unresolved frontier in one pass over the selected cut.
    ///
    /// A page can belong to at most one normalized target subtree, so ancestor lookup classifies
    /// the complete cut once. This is `O((N + T) * D * log T)` for `N` selected pages, `T`
    /// targets, and bounded hierarchy depth `D`, instead of scanning and retaining all `N` pages
    /// independently for every target.
    fn coarsen_surface_frontiers(
        &mut self,
        targets: BTreeMap<TerrainPageKey, (TerrainPageKey, BTreeSet<TerrainPageKey>)>,
    ) -> bool {
        if targets.is_empty() {
            return false;
        }

        // Multiple discontinuity components can converge on nested temporary owners. Normalize
        // them to the highest requested ancestor so the resulting target subtrees are disjoint,
        // retaining every blocker that must release that temporary owner.
        let target_keys = targets.keys().copied().collect::<BTreeSet<_>>();
        let mut normalized =
            BTreeMap::<TerrainPageKey, (TerrainPageKey, BTreeSet<TerrainPageKey>)>::new();
        for (target, (owner, blockers)) in &targets {
            let mut normalized_target = *target;
            let mut ancestor = target.parent();
            while let Some(candidate) = ancestor {
                if target_keys.contains(&candidate) {
                    normalized_target = candidate;
                }
                ancestor = candidate.parent();
            }
            let normalized_owner = targets
                .get(&normalized_target)
                .map_or(*owner, |(owner, _)| *owner);
            let entry = normalized
                .entry(normalized_target)
                .or_insert_with(|| (normalized_owner, BTreeSet::new()));
            if entry.0 != *owner {
                self.traversal_overflow = true;
                continue;
            }
            entry.1.extend(blockers.iter().copied());
        }

        // Classify each selected page exactly once by walking its bounded ancestor chain. Since
        // normalized targets are disjoint, the first match is also the only match.
        let normalized_keys = normalized.keys().copied().collect::<BTreeSet<_>>();
        let mut classified = BTreeMap::<TerrainPageKey, Vec<TerrainPageKey>>::new();
        for selected in self.selected.iter().copied() {
            let mut candidate = Some(selected);
            while let Some(key) = candidate {
                if normalized_keys.contains(&key) {
                    classified.entry(key).or_default().push(selected);
                    break;
                }
                candidate = key.parent();
            }
        }

        let mut replaced = BTreeSet::new();
        let mut inserted = BTreeMap::new();
        for (target, (owner, blockers)) in normalized {
            if target.ancestor_at(owner.level) != Some(owner) {
                continue;
            }
            let Some(classified_pages) = classified.get(&target) else {
                continue;
            };
            if classified_pages
                .iter()
                .any(|selected| self.surface_page_requires_exact(*selected))
                || classified_pages
                    .iter()
                    .any(|selected| self.selected_owners.get(selected).copied() != Some(owner))
            {
                continue;
            }
            if classified_pages.len() == 1 && classified_pages[0] == target {
                if !blockers.is_empty() {
                    self.next_balanced_selected_blockers
                        .entry(target)
                        .or_default()
                        .extend(blockers);
                }
                continue;
            }
            replaced.extend(classified_pages.iter().copied());
            inserted.insert(target, owner);
            self.next_balanced_selected.insert(target);
            if !blockers.is_empty() {
                self.next_balanced_selected_blockers
                    .entry(target)
                    .or_default()
                    .extend(blockers);
            }
        }
        if inserted.is_empty() {
            return false;
        }

        self.selected
            .retain(|selected| !replaced.contains(selected));
        self.selected_owners
            .retain(|selected, _| !replaced.contains(selected));
        for (target, owner) in &inserted {
            self.selected.push(*target);
            self.selected_owners.insert(*target, *owner);
        }
        let inserted = inserted.keys().copied().collect::<BTreeSet<_>>();
        self.next_refined
            .retain(|refined| !has_ancestor_in(*refined, &inserted));
        self.next_balanced_refined
            .retain(|refined| !has_ancestor_in(*refined, &inserted));
        true
    }

    fn selection_is_exact_active_root_partition(&self) -> bool {
        if self.selected.len() != self.selected_owners.len()
            || self
                .selected
                .iter()
                .any(|key| !self.selected_owners.contains_key(key))
        {
            return false;
        }
        let selected = self.selected.iter().copied().collect::<BTreeSet<_>>();
        if selected.len() != self.selected.len() {
            return false;
        }
        for key in &selected {
            let mut ancestor = key.parent();
            while let Some(candidate) = ancestor {
                if selected.contains(&candidate) {
                    return false;
                }
                ancestor = candidate.parent();
            }
        }
        let mut selected_measure = BTreeMap::<TerrainPageKey, u128>::new();
        for key in &selected {
            let Some(owner) = self.selected_owners.get(key).copied() else {
                return false;
            };
            if !self.visited_active_roots.contains(&owner)
                || key.is_surface() != owner.is_surface()
                || key.ancestor_at(owner.level) != Some(owner)
            {
                return false;
            }
            let branching = if owner.is_surface() { 4_u128 } else { 8_u128 };
            let Some(measure) = branching.checked_pow(u32::from(key.level)) else {
                return false;
            };
            let Some(total) = selected_measure
                .get(&owner)
                .copied()
                .unwrap_or(0)
                .checked_add(measure)
            else {
                return false;
            };
            selected_measure.insert(owner, total);
        }
        self.visited_active_roots.iter().copied().all(|root| {
            let branching = if root.is_surface() { 4_u128 } else { 8_u128 };
            let Some(expected) = branching.checked_pow(u32::from(root.level)) else {
                return false;
            };
            selected_measure.get(&root).copied().unwrap_or(0) == expected
        })
    }

    fn visit(&mut self, key: TerrainPageKey, root: bool, owner: TerrainPageKey) {
        // Frustum-cull only complete region roots. Once a root becomes a visible owner, every
        // refinement remains a complete octree partition of that root. Culling individual
        // descendants made `is_renderable` lie: the selected pages no longer covered the volume
        // whose parent owner was being replaced, which could expose gaps at the frustum edge.
        let Some(node) = self.hierarchy.nodes.get(&key).cloned() else {
            if root {
                self.ownerless_roots.push(key);
            }
            return;
        };
        let exact_surface = self.exact_surface_domain.intersects_page(key);
        let required_horizon_owner = self
            .presentation_envelope
            .is_some_and(|envelope| envelope.requires_horizon_owner(key));
        if root
            && !page_is_visible(node.bounds, self.view)
            && !exact_surface
            && !required_horizon_owner
        {
            return;
        }
        if self.visited_nodes >= self.hierarchy.capacity.max_traversal_nodes {
            self.traversal_overflow = true;
            if root {
                self.ownerless_roots.push(key);
            } else {
                self.select(key, owner);
            }
            return;
        }
        self.visited_nodes += 1;
        if !self.hierarchy.resident.contains_key(&key) {
            self.request(key);
            if root {
                self.ownerless_roots.push(key);
            }
            return;
        }
        if let Some(blockers) = self.prior_balanced_selected_blockers.get(&key).cloned() {
            let unresolved = blockers
                .into_iter()
                .filter(|blocker| {
                    !self
                        .hierarchy
                        .replacement_is_resident_and_coherent(*blocker)
                })
                .collect::<BTreeSet<_>>();
            if !unresolved.is_empty() {
                for blocker in &unresolved {
                    if let Some(children) = blocker.refinement_children() {
                        self.record_unavailable_replacement(*blocker, &children);
                    }
                }
                self.next_balanced_selected.insert(key);
                self.next_balanced_selected_blockers.insert(key, unresolved);
                self.select(key, owner);
                return;
            }
        }
        let threshold = if self.prior_refined.contains(&key) {
            self.view.coarsen_below_pixels
        } else {
            self.view.refine_above_pixels
        };
        let projected_error = projected_page_error_pixels(&node, self.view);
        let quality_visible = page_is_visible(node.bounds, self.view);
        let wants_more_detail = self.view.force_exact_leaves
            || exact_surface
            || (quality_visible && projected_error > threshold);
        if wants_more_detail && !node.has_children && key.is_surface() && key.level > 0 {
            if self.refinement_requests.len() < self.hierarchy.capacity.max_feedback_pages {
                self.refinement_requests.insert(key);
            } else {
                self.feedback_overflow = true;
            }
        }
        let wants_refinement = node.has_children && wants_more_detail;
        if wants_refinement
            && self.selected.len().saturating_add(
                key.refinement_children()
                    .map_or(TERRAIN_PAGE_MAX_CHILDREN, |children| children.len()),
            ) <= self.hierarchy.capacity.max_selected_pages
        {
            let Some(children) = key.refinement_children() else {
                self.select(key, owner);
                return;
            };
            if self.hierarchy.replacement_is_resident_and_coherent(key) {
                self.next_refined.insert(key);
                for child in children {
                    self.visit(child, false, owner);
                }
                return;
            }
            if children
                .iter()
                .all(|child| self.hierarchy.resident.contains_key(child))
            {
                self.incoherent_replacement_groups =
                    self.incoherent_replacement_groups.saturating_add(1);
            } else {
                for child in children {
                    if !self.hierarchy.resident.contains_key(&child) {
                        self.request(child);
                    }
                }
            }
        } else if wants_refinement && (self.view.force_exact_leaves || exact_surface) {
            // Exhausting the page budget while satisfying the exact player vicinity is a hard
            // publication failure. Projected-error and guard-band refinement are optional:
            // retaining their complete coarse owner is still a valid, renderable distant cut.
            self.selection_overflow = true;
        }
        self.select(key, owner);
    }

    fn select(&mut self, key: TerrainPageKey, owner: TerrainPageKey) {
        let Some(resident) = self.hierarchy.resident.get_mut(&key) else {
            return;
        };
        if self.selected.len() >= self.hierarchy.capacity.max_selected_pages {
            self.selection_overflow = true;
            return;
        }
        resident.last_selected_frame = self.frame;
        self.selected.push(key);
        self.selected_owners.insert(key, owner);
        self.selected_primitives = self
            .selected_primitives
            .saturating_add(resident.primitive_count);
        self.selected_encoded_bytes = self
            .selected_encoded_bytes
            .saturating_add(resident.encoded_bytes);
    }

    fn request(&mut self, key: TerrainPageKey) {
        let Some(node) = self.hierarchy.nodes.get(&key) else {
            return;
        };
        if self.requests.len() >= self.hierarchy.capacity.max_feedback_pages {
            self.feedback_overflow = true;
            return;
        }
        self.requests.insert(TerrainPageTransferIdentity {
            key,
            revision: node.revision,
            content_fingerprint: node.content_fingerprint,
        });
    }
}

fn page_primitive_count(page: &TerrainPageV1) -> usize {
    match &page.representation {
        TerrainPageRepresentation::SteppedSurfaceResidual(_)
        | TerrainPageRepresentation::SparseVoxelBrick(_) => {
            reconstruct_exact_terrain_surface(page).map_or(usize::MAX, |quads| quads.len())
        }
        TerrainPageRepresentation::SurfaceCluster(quads) => quads.len(),
        TerrainPageRepresentation::TriangleCluster(cluster) => cluster.triangles.len(),
        TerrainPageRepresentation::HeightfieldGrid(grid) => {
            let ground =
                TERRAIN_PAGE_EDGE_SAMPLES as usize * TERRAIN_PAGE_EDGE_SAMPLES as usize * 2;
            let edge = TERRAIN_PAGE_EDGE_SAMPLES as usize + 1;
            let water = (0..TERRAIN_PAGE_EDGE_SAMPLES as usize)
                .flat_map(|z| {
                    (0..TERRAIN_PAGE_EDGE_SAMPLES as usize).map(move |x| {
                        let samples = [
                            x + z * edge,
                            x + 1 + z * edge,
                            x + 1 + (z + 1) * edge,
                            x + (z + 1) * edge,
                        ];
                        usize::from(
                            [samples[0], samples[2], samples[1]]
                                .into_iter()
                                .all(|index| grid.water_heights[index] != i32::MIN),
                        ) + usize::from(
                            [samples[0], samples[3], samples[2]]
                                .into_iter()
                                .all(|index| grid.water_heights[index] != i32::MIN),
                        )
                    })
                })
                .sum::<usize>();
            ground + water
        }
    }
}

fn projected_page_error_pixels(node: &TerrainHierarchyNode, view: VirtualTerrainView) -> f64 {
    if node.errors.unresolved_topology {
        return f64::INFINITY;
    }
    let positional_error_millivoxels = node
        .errors
        .geometric_millivoxels
        .max(node.errors.silhouette_millivoxels)
        .max(node.errors.material_boundary_millivoxels);
    let positional_error_metres = f64::from(positional_error_millivoxels) * 0.000_1;
    let distance =
        distance_to_page_metres(node.bounds, view.camera_position_metres).max(view.near_metres);
    let projection_scale =
        f64::from(view.viewport_height_pixels) / (2.0 * (view.vertical_fov_radians * 0.5).tan());
    let positional_pixels = positional_error_metres * projection_scale / distance;
    let normal_pixels = f64::from(node.errors.normal_milliradians)
        * 0.001
        * NORMAL_ERROR_PIXELS_PER_RADIAN
        * view.wet_specular_sensitivity;
    positional_pixels.max(normal_pixels)
}

fn distance_to_page_metres(bounds: voxels_world::VoxelBounds, point: [f64; 3]) -> f64 {
    let minimum = bounds.min.as_array().map(|value| f64::from(value) * 0.1);
    let maximum = bounds.max.as_array().map(|value| f64::from(value) * 0.1);
    (0..3)
        .map(|axis| {
            let distance = if point[axis] < minimum[axis] {
                minimum[axis] - point[axis]
            } else if point[axis] > maximum[axis] {
                point[axis] - maximum[axis]
            } else {
                0.0
            };
            distance * distance
        })
        .sum::<f64>()
        .sqrt()
}

fn page_is_visible(bounds: voxels_world::VoxelBounds, view: VirtualTerrainView) -> bool {
    let minimum = bounds.min.as_array().map(|value| f64::from(value) * 0.1);
    let maximum = bounds.max.as_array().map(|value| f64::from(value) * 0.1);
    let center = [
        (minimum[0] + maximum[0]) * 0.5,
        (minimum[1] + maximum[1]) * 0.5,
        (minimum[2] + maximum[2]) * 0.5,
    ];
    let radius = length([
        maximum[0] - center[0],
        maximum[1] - center[1],
        maximum[2] - center[2],
    ]);
    let forward = normalize(view.camera_forward);
    let mut right = cross(forward, [0.0, 1.0, 0.0]);
    if length_squared(right) <= f64::EPSILON {
        right = [1.0, 0.0, 0.0];
    } else {
        right = normalize(right);
    }
    let up = normalize(cross(right, forward));
    let relative = subtract(center, view.camera_position_metres);
    let depth = dot(relative, forward);
    if depth + radius < view.near_metres || depth - radius > view.far_metres {
        return false;
    }
    let tangent_vertical = (view.vertical_fov_radians * 0.5).tan();
    let tangent_horizontal = tangent_vertical * view.aspect_ratio;
    let horizontal_plane_scale = (1.0 + tangent_horizontal * tangent_horizontal).sqrt();
    let vertical_plane_scale = (1.0 + tangent_vertical * tangent_vertical).sqrt();
    dot(relative, right).abs() <= depth * tangent_horizontal + radius * horizontal_plane_scale
        && dot(relative, up).abs() <= depth * tangent_vertical + radius * vertical_plane_scale
}

fn cut_fingerprint(selected: &[TerrainPageKey], hierarchy: &VirtualTerrainHierarchy) -> u64 {
    let mut fingerprint = FINGERPRINT_OFFSET;
    for key in selected {
        fingerprint = fingerprint_byte(fingerprint, key.level);
        for component in key.coord {
            for byte in component.to_le_bytes() {
                fingerprint = fingerprint_byte(fingerprint, byte);
            }
        }
        if let Some(page) = hierarchy.resident.get(key) {
            for byte in page.page.revision.to_le_bytes() {
                fingerprint = fingerprint_byte(fingerprint, byte);
            }
            for byte in page.page.content_fingerprint {
                fingerprint = fingerprint_byte(fingerprint, byte);
            }
        }
    }
    fingerprint
}

fn cut_state_fingerprint(
    mut fingerprint: u64,
    ownerless_roots: &[TerrainPageKey],
    feedback_overflow: bool,
    selection_overflow: bool,
    traversal_overflow: bool,
    incoherent_replacement_groups: usize,
    exact_surface_lod_discontinuities: usize,
) -> u64 {
    // Selected owners alone do not identify an incomplete desired plan: two traversals may retain
    // the same fallback owners while missing different active roots. Keep the exact sorted blocker
    // set and every publication-relevant failure state in the identity passed to GPU certification.
    fingerprint = fingerprint_byte(fingerprint, 0xff);
    for key in ownerless_roots {
        fingerprint = fingerprint_byte(fingerprint, key.level);
        for component in key.coord {
            for byte in component.to_le_bytes() {
                fingerprint = fingerprint_byte(fingerprint, byte);
            }
        }
    }
    for byte in (ownerless_roots.len() as u64).to_le_bytes() {
        fingerprint = fingerprint_byte(fingerprint, byte);
    }
    let flags = u8::from(feedback_overflow)
        | (u8::from(selection_overflow) << 1)
        | (u8::from(traversal_overflow) << 2);
    fingerprint = fingerprint_byte(fingerprint, flags);
    for value in [
        incoherent_replacement_groups as u64,
        exact_surface_lod_discontinuities as u64,
    ] {
        for byte in value.to_le_bytes() {
            fingerprint = fingerprint_byte(fingerprint, byte);
        }
    }
    fingerprint
}

fn fingerprint_byte(fingerprint: u64, byte: u8) -> u64 {
    (fingerprint ^ u64::from(byte)).wrapping_mul(FINGERPRINT_PRIME)
}

fn subtract(left: [f64; 3], right: [f64; 3]) -> [f64; 3] {
    [left[0] - right[0], left[1] - right[1], left[2] - right[2]]
}

fn dot(left: [f64; 3], right: [f64; 3]) -> f64 {
    left[0] * right[0] + left[1] * right[1] + left[2] * right[2]
}

fn cross(left: [f64; 3], right: [f64; 3]) -> [f64; 3] {
    [
        left[1] * right[2] - left[2] * right[1],
        left[2] * right[0] - left[0] * right[2],
        left[0] * right[1] - left[1] * right[0],
    ]
}

fn length_squared(vector: [f64; 3]) -> f64 {
    dot(vector, vector)
}

fn length(vector: [f64; 3]) -> f64 {
    length_squared(vector).sqrt()
}

fn normalize(vector: [f64; 3]) -> [f64; 3] {
    let inverse = length(vector).recip();
    [
        vector[0] * inverse,
        vector[1] * inverse,
        vector[2] * inverse,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use voxels_world::{
        Material, SurfaceRegion, SurfaceSample, TerrainErrorBounds, TerrainHierarchyDirectoryV1,
        VoxelCoord, build_exact_cluster_terrain_parent, build_exact_terrain_page,
        build_sampled_heightfield_terrain_page,
    };

    fn identity() -> WorldSourceIdentityHash {
        WorldSourceIdentityHash::from_bytes([0x95; 32])
    }

    fn hierarchy_pages() -> Vec<TerrainPageV1> {
        let root_key = TerrainPageKey {
            level: 1,
            coord: [-1, 0, -1],
        };
        let children = root_key
            .children()
            .unwrap()
            .into_iter()
            .map(|key| {
                build_exact_terrain_page(identity(), key, 7, |coord| {
                    if coord.y <= 10 {
                        Material::Stone
                    } else {
                        Material::Air
                    }
                })
                .unwrap()
            })
            .collect::<Vec<_>>();
        let root = build_exact_cluster_terrain_parent(root_key, 7, &children).unwrap();
        std::iter::once(root).chain(children).collect()
    }

    fn view(force_exact_leaves: bool) -> VirtualTerrainView {
        VirtualTerrainView {
            camera_position_metres: [-3.2, 3.2, 8.0],
            camera_forward: [0.0, 0.0, -1.0],
            vertical_fov_radians: 1.0,
            aspect_ratio: 16.0 / 9.0,
            viewport_height_pixels: 1080,
            near_metres: 0.1,
            far_metres: 1_000.0,
            refine_above_pixels: 0.65,
            coarsen_below_pixels: 0.35,
            wet_specular_sensitivity: 1.0,
            force_exact_leaves,
        }
    }

    fn exact_domain(view: VirtualTerrainView, radius_metres: f64) -> ExactSurfaceDomain {
        ExactSurfaceDomain::swept_horizontal_capsule(
            view.camera_position_metres,
            view.camera_position_metres,
            radius_metres,
            16_384,
        )
    }

    fn select_cut(
        hierarchy: &mut VirtualTerrainHierarchy,
        view: VirtualTerrainView,
    ) -> Result<VirtualTerrainCut, VirtualTerrainError> {
        hierarchy.select_cut(view, &exact_domain(view, 0.0))
    }

    fn default_exact_domain() -> &'static ExactSurfaceDomain {
        static DOMAIN: std::sync::LazyLock<ExactSurfaceDomain> = std::sync::LazyLock::new(|| {
            let view = view(false);
            exact_domain(view, 0.0)
        });
        &DOMAIN
    }

    fn cut_with_selected(selected_pages: Vec<TerrainPageKey>) -> VirtualTerrainCut {
        VirtualTerrainCut {
            selected_pages,
            requested_pages: Vec::new(),
            refinement_roots: Vec::new(),
            ownerless_roots: Vec::new(),
            fingerprint: 1,
            visited_nodes: 1,
            selected_primitives: 1,
            selected_encoded_bytes: 1,
            feedback_overflow: false,
            selection_overflow: false,
            traversal_overflow: false,
            incoherent_replacement_groups: 0,
            exact_surface_lod_discontinuities: 0,
        }
    }

    fn complete_envelope_cut(envelope: &PresentationEnvelope) -> VirtualTerrainCut {
        fn partition(
            key: TerrainPageKey,
            exact: &BTreeSet<TerrainPageKey>,
            selected: &mut Vec<TerrainPageKey>,
        ) {
            let intersects_exact = exact
                .iter()
                .any(|leaf| leaf.ancestor_at(key.level) == Some(key));
            if key.level == 0 || !intersects_exact {
                selected.push(key);
                return;
            }
            for child in key.refinement_children().unwrap() {
                partition(child, exact, selected);
            }
        }

        let exact = envelope
            .exact_surface_domain()
            .required_leaves()
            .collect::<BTreeSet<_>>();
        let mut selected = Vec::new();
        for root in envelope.required_horizon_roots() {
            partition(root, &exact, &mut selected);
        }
        selected.sort_unstable();
        cut_with_selected(selected)
    }

    #[test]
    fn presentation_loci_are_half_open_snapped_and_overlap_at_negative_coordinates() {
        let first = PresentationLocus::containing_position([-0.1, 0.0, -0.1]).unwrap();
        let next = PresentationLocus::containing_position([3.2, 0.0, -0.1]).unwrap();

        assert_eq!(first.minimum_leaf(), [-3, -3]);
        assert_eq!(first.maximum_leaf_exclusive(), [1, 1]);
        assert_eq!(next.minimum_leaf(), [-1, -3]);
        assert_eq!(next.maximum_leaf_exclusive(), [3, 1]);
        assert!(first.contains_position([-9.6, 0.0, -9.6]));
        assert!(!first.contains_position([3.2, 0.0, 0.0]));
        assert!(next.contains_position([3.2, 0.0, 0.0]));

        let clamped = first.clamp_position([3.2, 7.0, -20.0]);
        assert!(first.contains_position(clamped));
        assert_eq!(clamped[1], 7.0);
        assert_eq!(clamped[2], -9.6);
        assert!(clamped[0] < 3.2);
    }

    #[test]
    fn presentation_envelope_cache_depends_on_locus_not_subcell_motion_or_velocity() {
        let mut cache = PresentationEnvelopeCache::default();
        let first = cache.resolve([0.1, 0.0, 0.1], 12.8, 3_200.0, 16_384, 16);
        let moved_inside = cache.resolve([2.9, 40.0, 2.9], 12.8, 3_200.0, 16_384, 16);
        let adjacent = cache.resolve([9.7, 0.0, 0.1], 12.8, 3_200.0, 16_384, 16);

        assert!(first.is_complete());
        assert!(first.shares_storage_with(&moved_inside));
        assert!(!first.shares_storage_with(&adjacent));
        assert_eq!(cache.rebuilds(), 2);
        assert_eq!(first.required_horizon_root_count(), 4);
    }

    #[test]
    fn every_stationary_core_inside_the_locus_is_a_subset_of_safety_leaves() {
        let mut cache = PresentationEnvelopeCache::default();
        let envelope = cache.resolve([0.1, 0.0, 0.1], 12.8, 3_200.0, 16_384, 16);
        let safety = envelope
            .exact_surface_domain()
            .required_leaves()
            .collect::<BTreeSet<_>>();
        let [minimum, maximum] = envelope.locus().unwrap().horizontal_bounds_metres();
        let probes = [
            [minimum[0], 0.0, minimum[1]],
            [maximum[0] - 1.0e-6, 0.0, minimum[1]],
            [minimum[0], 0.0, maximum[1] - 1.0e-6],
            [maximum[0] - 1.0e-6, 0.0, maximum[1] - 1.0e-6],
            [
                (minimum[0] + maximum[0]) * 0.5,
                0.0,
                (minimum[1] + maximum[1]) * 0.5,
            ],
        ];
        for probe in probes {
            let core = ExactSurfaceDomain::swept_horizontal_capsule(probe, probe, 12.8, 16_384);
            assert!(
                core.required_leaves().all(|leaf| safety.contains(&leaf)),
                "stationary core at {probe:?} escaped the locus safety set",
            );
        }
    }

    #[test]
    fn presentation_cut_requires_every_exact_leaf_and_explicit_horizon_root() {
        let envelope = PresentationEnvelopeCache::default().resolve(
            [-0.1, 0.0, -0.1],
            3.2,
            3_200.0,
            16_384,
            16,
        );
        let complete = complete_envelope_cut(&envelope);
        assert!(complete.covers_presentation_envelope(&envelope));

        let missing_leaf = envelope
            .exact_surface_domain()
            .required_leaves()
            .next()
            .unwrap();
        let mut incomplete_exact = complete.clone();
        incomplete_exact
            .selected_pages
            .retain(|selected| *selected != missing_leaf);
        assert!(!incomplete_exact.covers_presentation_envelope(&envelope));

        let missing_root = envelope.required_horizon_roots().next().unwrap();
        let mut incomplete_horizon = complete.clone();
        incomplete_horizon.selected_pages.retain(|selected| {
            selected.ancestor_at(TERRAIN_COVERAGE_ROOT_LEVEL) != Some(missing_root)
        });
        assert!(!incomplete_horizon.covers_presentation_envelope(&envelope));

        let mut equal_count_swap = complete;
        let removed = equal_count_swap
            .selected_pages
            .iter()
            .position(|selected| *selected == missing_leaf)
            .unwrap();
        equal_count_swap.selected_pages[removed] =
            TerrainPageKey::surface(TERRAIN_COVERAGE_ROOT_LEVEL, i32::MAX, i32::MAX);
        equal_count_swap.selected_pages.sort_unstable();
        assert!(!equal_count_swap.covers_presentation_envelope(&envelope));
    }

    #[test]
    fn presentation_envelope_fails_closed_when_exact_or_horizon_bounds_overflow() {
        let exact_overflow =
            PresentationEnvelopeCache::default().resolve([0.0, 0.0, 0.0], 12.8, 3_200.0, 1, 16);
        let horizon_overflow =
            PresentationEnvelopeCache::default().resolve([0.0, 0.0, 0.0], 12.8, 3_200.0, 16_384, 1);

        assert!(!exact_overflow.is_complete());
        assert!(!horizon_overflow.is_complete());
    }

    #[test]
    fn presentation_selection_visits_required_horizon_owners_behind_the_camera() {
        let behind_root = TerrainPageKey::surface(TERRAIN_COVERAGE_ROOT_LEVEL, -2, 0);
        let (directory, pages) = surface_segment(behind_root);
        let mut hierarchy =
            VirtualTerrainHierarchy::new(VirtualTerrainCapacity::DEVELOPMENT_128_MIB).unwrap();
        hierarchy.register_region_directory(&directory).unwrap();
        for page in pages {
            hierarchy.install_page(page).unwrap();
        }

        let mut east_facing_view = view(false);
        east_facing_view.camera_position_metres = [10.0, 20.0, 1_000.0];
        east_facing_view.camera_forward = [1.0, 0.0, 0.0];
        east_facing_view.far_metres = 100.0;
        let envelope = PresentationEnvelopeCache::default().resolve(
            east_facing_view.camera_position_metres,
            0.0,
            3_300.0,
            64,
            16,
        );
        assert!(envelope.is_complete());
        assert!(
            envelope
                .required_horizon_roots()
                .any(|root| root == behind_root),
            "the frozen horizon must include the adjacent root behind the east-facing camera",
        );
        assert!(
            !page_is_visible(
                hierarchy.directory_node(behind_root).unwrap().bounds,
                east_facing_view,
            ),
            "the fixture must actually be rejected by ordinary frustum traversal",
        );

        let ordinary = hierarchy
            .select_cut(east_facing_view, envelope.exact_surface_domain())
            .unwrap();
        assert!(!ordinary.selected_pages.contains(&behind_root));

        let presentation = hierarchy
            .select_presentation_cut(east_facing_view, &envelope)
            .unwrap();
        assert!(presentation.selected_pages.contains(&behind_root));
        assert!(
            behind_root
                .refinement_children()
                .unwrap()
                .into_iter()
                .all(|child| !presentation.selected_pages.contains(&child)),
            "horizon ownership must not spend projected-error quality on pages behind the camera",
        );
        assert!(
            !presentation.ownerless_roots.contains(&behind_root),
            "the selected behind-camera horizon root is a real owner, not missing coverage debt",
        );
        assert!(
            !presentation.covers_presentation_envelope(&envelope),
            "the still-unregistered forward root must prevent publication",
        );
    }

    #[test]
    fn ownership_only_view_is_valid_and_disables_optional_pixel_refinement() {
        let ordinary = view(false);
        let ownership = ordinary.ownership_only();

        assert!(ownership.validates());
        assert_eq!(
            ownership.camera_position_metres,
            ordinary.camera_position_metres
        );
        assert_eq!(ownership.camera_forward, ordinary.camera_forward);
        assert_eq!(ownership.force_exact_leaves, ordinary.force_exact_leaves);
        assert!(ownership.coarsen_below_pixels > ordinary.refine_above_pixels);
        assert!(ownership.refine_above_pixels > ownership.coarsen_below_pixels);
    }

    #[test]
    fn cut_identity_includes_exact_ownerless_roots_and_failure_state() {
        let base = 0x1234_5678_9abc_def0;
        let first = TerrainPageKey::surface(3, -2, 7);
        let second = TerrainPageKey::surface(3, 3, 7);
        let first_missing = cut_state_fingerprint(base, &[first], false, false, false, 0, 0);
        let second_missing = cut_state_fingerprint(base, &[second], false, false, false, 0, 0);
        let overflowed = cut_state_fingerprint(base, &[first], false, true, false, 0, 0);

        assert_ne!(first_missing, second_missing);
        assert_ne!(first_missing, overflowed);
    }

    #[test]
    fn surface_cut_rejects_skipped_neighbor_levels_without_geometry_inspection() {
        let fine = TerrainPageKey::surface(0, 7, 0);
        let adjacent_level_one = TerrainPageKey::surface(1, 4, 0);
        let adjacent_level_three = TerrainPageKey::surface(3, 1, 0);

        assert_eq!(
            exact_surface_lod_discontinuity_edges(&[fine, adjacent_level_one]),
            0
        );
        assert_eq!(
            exact_surface_lod_discontinuity_edges(&[fine, adjacent_level_three]),
            1
        );
        assert_eq!(
            exact_surface_lod_discontinuity_edges(&[
                TerrainPageKey::surface(2, -1, -4),
                TerrainPageKey::surface(4, -1, -2),
            ]),
            1,
            "negative-coordinate transition edges must use Euclidean ancestry",
        );
    }

    fn hierarchy() -> (VirtualTerrainHierarchy, Vec<TerrainPageV1>) {
        let pages = hierarchy_pages();
        let directory = TerrainHierarchyDirectoryV1::from_pages(&pages).unwrap();
        let mut hierarchy =
            VirtualTerrainHierarchy::new(VirtualTerrainCapacity::DEVELOPMENT_128_MIB).unwrap();
        hierarchy.register_region_directory(&directory).unwrap();
        (hierarchy, pages)
    }

    fn surface_page(key: TerrainPageKey) -> TerrainPageV1 {
        let [[minimum_x, minimum_z], _] = key.horizontal_bounds().unwrap();
        let stride = 1_i32 << u32::from(key.level);
        let edge = TERRAIN_PAGE_EDGE_SAMPLES as usize + 1;
        let samples = (0..edge)
            .flat_map(|z| {
                (0..edge).map(move |x| {
                    let world_x = minimum_x + i32::try_from(x).unwrap() * stride;
                    let world_z = minimum_z + i32::try_from(z).unwrap() * stride;
                    SurfaceSample {
                        height: world_x.div_euclid(5) + world_z.div_euclid(7),
                        material: Material::Stone,
                        water_level: None,
                        region: SurfaceRegion::VerdantForest,
                        moisture: 0.5,
                        temperature: 0.5,
                        ridge: 0.0,
                        route: None,
                    }
                })
            })
            .collect::<Vec<_>>();
        let boundary_midpoints = if key.level == 0 {
            Vec::new()
        } else {
            let midpoint = stride / 2;
            let [[_, _], [maximum_x, maximum_z]] = key.horizontal_bounds().unwrap();
            [
                (minimum_x, true),
                (maximum_x, true),
                (minimum_z, false),
                (maximum_z, false),
            ]
            .into_iter()
            .flat_map(|(fixed, x_fixed)| {
                (0..TERRAIN_PAGE_EDGE_SAMPLES as i32).map(move |offset| {
                    let tangent =
                        if x_fixed { minimum_z } else { minimum_x } + offset * stride + midpoint;
                    let (world_x, world_z) = if x_fixed {
                        (fixed, tangent)
                    } else {
                        (tangent, fixed)
                    };
                    SurfaceSample {
                        height: world_x.div_euclid(5) + world_z.div_euclid(7),
                        material: Material::Stone,
                        water_level: None,
                        region: SurfaceRegion::VerdantForest,
                        moisture: 0.5,
                        temperature: 0.5,
                        ridge: 0.0,
                        route: None,
                    }
                })
            })
            .collect()
        };
        let error = 1_000_u32 << u32::from(key.level);
        build_sampled_heightfield_terrain_page(
            identity(),
            key,
            7,
            &samples,
            &boundary_midpoints,
            TerrainErrorBounds {
                geometric_millivoxels: error,
                silhouette_millivoxels: error,
                material_boundary_millivoxels: 0,
                normal_milliradians: 0,
                unresolved_topology: false,
            },
        )
        .unwrap()
    }

    fn insert_unregistered_resident(hierarchy: &mut VirtualTerrainHierarchy, page: TerrainPageV1) {
        let encoded_bytes = encode_terrain_page(&page).unwrap().len();
        let primitive_count = page_primitive_count(&page);
        hierarchy.resident.insert(
            page.key,
            ResidentPage {
                page,
                encoded_bytes,
                primitive_count,
                last_selected_frame: 0,
            },
        );
    }

    fn cut_builder_for_owned_selection<'a>(
        hierarchy: &'a mut VirtualTerrainHierarchy,
        prior_refined: &'a BTreeSet<TerrainPageKey>,
        exact_surface_domain: &'a ExactSurfaceDomain,
        selected: Vec<TerrainPageKey>,
        selected_owners: BTreeMap<TerrainPageKey, TerrainPageKey>,
    ) -> CutBuilder<'a> {
        let visited_active_roots = selected_owners.values().copied().collect();
        CutBuilder {
            hierarchy,
            view: view(false),
            exact_surface_domain,
            presentation_envelope: None,
            frame: 1,
            prior_refined,
            prior_balanced_selected_blockers: BTreeMap::new(),
            next_refined: prior_refined.clone(),
            next_balanced_refined: BTreeSet::new(),
            next_balanced_selected: BTreeSet::new(),
            next_balanced_selected_blockers: BTreeMap::new(),
            selected,
            selected_owners,
            visited_active_roots,
            requests: BTreeSet::new(),
            refinement_requests: BTreeSet::new(),
            ownerless_roots: Vec::new(),
            visited_nodes: 0,
            selected_primitives: 0,
            selected_encoded_bytes: 0,
            feedback_overflow: false,
            selection_overflow: false,
            traversal_overflow: false,
            incoherent_replacement_groups: 0,
        }
    }

    #[test]
    fn surface_balance_refines_the_coarse_side_without_sacrificing_exact_pages() {
        let fine = TerrainPageKey::surface(0, 3, 0);
        let coarse = TerrainPageKey::surface(2, 1, 0);
        let children = coarse.refinement_children().unwrap();
        let selected = vec![fine, coarse];
        assert!(
            exact_surface_lod_discontinuity_edges(&selected) > 0,
            "the fixture must contain a skipped intermediate surface level"
        );

        let mut hierarchy =
            VirtualTerrainHierarchy::new(VirtualTerrainCapacity::DEVELOPMENT_128_MIB).unwrap();
        for key in selected.iter().copied().chain(children.iter().copied()) {
            insert_unregistered_resident(&mut hierarchy, surface_page(key));
        }
        hierarchy.coherent_replacements.insert(coarse);
        let prior_refined = BTreeSet::new();
        let selected_owners = BTreeMap::from([(fine, fine), (coarse, coarse)]);
        let mut builder = cut_builder_for_owned_selection(
            &mut hierarchy,
            &prior_refined,
            default_exact_domain(),
            selected,
            selected_owners,
        );

        builder.balance_surface_lod();

        assert!(builder.selected.contains(&fine));
        assert!(!builder.selected.contains(&coarse));
        assert!(
            children
                .iter()
                .all(|child| builder.selected.contains(child))
        );
        assert_eq!(exact_surface_lod_discontinuity_edges(&builder.selected), 0);
        assert_eq!(builder.next_refined, BTreeSet::from([coarse]));
        assert_eq!(builder.next_balanced_refined, BTreeSet::from([coarse]));
        assert!(builder.next_balanced_selected.is_empty());
        assert!(!builder.traversal_overflow);
    }

    #[test]
    fn surface_balance_coarsens_fine_side_until_coarse_children_are_complete() {
        let fine = TerrainPageKey::surface(0, 3, 0);
        let temporary_fine_owner = fine.ancestor_at(1).unwrap();
        let coarse = TerrainPageKey::surface(2, 1, 0);
        let children = coarse.refinement_children().unwrap();
        let coarse_page = surface_page(coarse);
        let pages = std::iter::once(coarse_page.clone())
            .chain(children.iter().copied().map(surface_page))
            .collect::<Vec<_>>();
        let directory =
            TerrainHierarchyDirectoryV1::from_surface_refinement_pages(coarse, &pages).unwrap();
        let mut hierarchy =
            VirtualTerrainHierarchy::new(VirtualTerrainCapacity::DEVELOPMENT_128_MIB).unwrap();
        hierarchy.register_region_directory(&directory).unwrap();
        hierarchy.install_page(coarse_page).unwrap();
        insert_unregistered_resident(&mut hierarchy, surface_page(fine));
        insert_unregistered_resident(&mut hierarchy, surface_page(temporary_fine_owner));
        let selected = vec![fine, coarse];
        let prior_refined = BTreeSet::new();
        let selected_owners = BTreeMap::from([(fine, temporary_fine_owner), (coarse, coarse)]);
        let mut builder = cut_builder_for_owned_selection(
            &mut hierarchy,
            &prior_refined,
            default_exact_domain(),
            selected.clone(),
            selected_owners,
        );

        builder.balance_surface_lod();

        assert!(builder.selected.contains(&temporary_fine_owner));
        assert!(builder.selected.contains(&coarse));
        assert!(!builder.selected.contains(&fine));
        assert_eq!(
            builder
                .requests
                .iter()
                .map(|identity| identity.key)
                .collect::<BTreeSet<_>>(),
            children.into_iter().collect()
        );
        assert_eq!(exact_surface_lod_discontinuity_edges(&builder.selected), 0);
        assert!(builder.next_refined.is_empty());
        assert!(builder.next_balanced_refined.is_empty());
        assert_eq!(
            builder.next_balanced_selected,
            BTreeSet::from([temporary_fine_owner])
        );
    }

    #[test]
    fn long_unresolved_frontier_retains_only_linear_causal_blocker_state() {
        const FRONTIER_EDGES: i32 = 64;
        let mut hierarchy =
            VirtualTerrainHierarchy::new(VirtualTerrainCapacity::DEVELOPMENT_128_MIB).unwrap();
        let edges = (0..FRONTIER_EDGES)
            .map(|index| {
                let fine = TerrainPageKey::surface(0, index.saturating_mul(4), 0);
                let owner = fine.parent().unwrap();
                let coarse = TerrainPageKey::surface(2, 100 + index, 0);
                insert_unregistered_resident(&mut hierarchy, surface_page(fine));
                insert_unregistered_resident(&mut hierarchy, surface_page(owner));
                (fine, owner, coarse)
            })
            .collect::<Vec<_>>();
        let selected = edges.iter().map(|(fine, _, _)| *fine).collect::<Vec<_>>();
        let selected_owners = edges
            .iter()
            .map(|(fine, owner, _)| (*fine, *owner))
            .collect::<BTreeMap<_, _>>();
        let prior_refined = BTreeSet::new();
        let mut builder = cut_builder_for_owned_selection(
            &mut hierarchy,
            &prior_refined,
            default_exact_domain(),
            selected,
            selected_owners,
        );
        let unresolved = vec![
            edges
                .iter()
                .map(|(fine, _, coarse)| (*fine, *coarse))
                .collect::<Vec<_>>(),
        ];

        let targets = builder.surface_coarsening_targets(unresolved);
        assert_eq!(targets.len(), FRONTIER_EDGES as usize);
        assert!(builder.coarsen_surface_frontiers(targets));

        assert_eq!(
            builder.next_balanced_selected_blockers.len(),
            FRONTIER_EDGES as usize
        );
        assert_eq!(
            builder
                .next_balanced_selected_blockers
                .values()
                .map(BTreeSet::len)
                .sum::<usize>(),
            FRONTIER_EDGES as usize,
            "causal blocker links must grow with seam edges, not edges squared"
        );
        assert!(
            builder
                .next_balanced_selected_blockers
                .values()
                .all(|blockers| blockers.len() == 1)
        );
    }

    #[test]
    fn disjoint_surface_frontiers_progress_independently() {
        let fine_owner_ready = TerrainPageKey::surface(1, 1, 0);
        let coarse_ready = TerrainPageKey::surface(2, 1, 0);
        let fine_owner_missing = TerrainPageKey::surface(1, 51, 0);
        let coarse_missing = TerrainPageKey::surface(2, 26, 0);
        let fine_ready = fine_owner_ready.refinement_children().unwrap();
        let ready_children = coarse_ready.refinement_children().unwrap();
        let fine_missing = fine_owner_missing.refinement_children().unwrap();
        let missing_children = coarse_missing.refinement_children().unwrap();
        let mut hierarchy =
            VirtualTerrainHierarchy::new(VirtualTerrainCapacity::DEVELOPMENT_128_MIB).unwrap();
        for root in [
            fine_owner_ready,
            coarse_ready,
            fine_owner_missing,
            coarse_missing,
        ] {
            let (directory, pages) = surface_segment(root);
            hierarchy.register_region_directory(&directory).unwrap();
            for page in pages {
                if root != coarse_missing || page.key == root {
                    hierarchy.install_page(page).unwrap();
                }
            }
        }
        assert!(
            hierarchy.replacement_is_resident_and_coherent(coarse_ready),
            "the ready frontier fixture must have a certified complete replacement"
        );
        assert!(
            !hierarchy.replacement_is_resident_and_coherent(coarse_missing),
            "the missing frontier fixture must remain unresolved"
        );
        let selected = fine_ready
            .iter()
            .copied()
            .chain([coarse_ready])
            .chain(fine_missing.iter().copied())
            .chain([coarse_missing])
            .collect::<Vec<_>>();
        let mut reversed = selected.clone();
        reversed.reverse();
        assert_eq!(
            surface_lod_discontinuity_components(&surface_lod_discontinuity_pairs(&selected)),
            surface_lod_discontinuity_components(&surface_lod_discontinuity_pairs(&reversed)),
            "frontier construction and page-key tie breaks must not depend on traversal order"
        );
        let selected_owners = fine_ready
            .iter()
            .copied()
            .map(|key| (key, fine_owner_ready))
            .chain([(coarse_ready, coarse_ready)])
            .chain(
                fine_missing
                    .iter()
                    .copied()
                    .map(|key| (key, fine_owner_missing)),
            )
            .chain([(coarse_missing, coarse_missing)])
            .collect::<BTreeMap<_, _>>();
        let prior_refined = BTreeSet::new();
        let exact_page = coarse_ready;
        let [[minimum_x, minimum_z], [maximum_x, maximum_z]] =
            exact_page.horizontal_bounds().unwrap();
        let exact_position = [
            f64::from(minimum_x + maximum_x) * 0.05,
            0.0,
            f64::from(minimum_z + maximum_z) * 0.05,
        ];
        let exact_surface_domain =
            ExactSurfaceDomain::swept_horizontal_capsule(exact_position, exact_position, 0.1, 16);
        let mut builder = cut_builder_for_owned_selection(
            &mut hierarchy,
            &prior_refined,
            &exact_surface_domain,
            selected,
            selected_owners,
        );
        builder.view.camera_position_metres = exact_position;
        let ready_component = surface_lod_discontinuity_components(
            &surface_lod_discontinuity_pairs(&builder.selected),
        )
        .into_iter()
        .find(|component| component.iter().any(|(_, coarse)| *coarse == coarse_ready))
        .unwrap();
        assert!(
            builder.surface_component_requires_exact(&ready_component),
            "coarse-side exact vicinity must receive the same priority as a fine endpoint"
        );

        builder.balance_surface_lod();

        assert!(
            fine_ready.iter().all(|key| builder.selected.contains(key)),
            "the ready component must preserve its existing fine side"
        );
        assert!(!builder.selected.contains(&coarse_ready));
        assert!(
            ready_children
                .iter()
                .all(|key| builder.selected.contains(key)),
            "the coherent coarse side must refine even though a disjoint frontier is missing"
        );
        assert!(builder.selected.contains(&fine_owner_missing));
        assert!(
            fine_missing
                .iter()
                .all(|key| !builder.selected.contains(key)),
            "only the unresolved component should use its temporary coarse owner"
        );
        assert!(builder.selected.contains(&coarse_missing));
        assert_eq!(
            builder
                .requests
                .iter()
                .map(|identity| identity.key)
                .collect::<BTreeSet<_>>(),
            missing_children.into_iter().collect()
        );
        assert_eq!(
            builder.next_balanced_refined,
            BTreeSet::from([coarse_ready])
        );
        assert_eq!(
            builder.next_balanced_selected,
            BTreeSet::from([fine_owner_missing])
        );
        assert_eq!(exact_surface_lod_discontinuity_edges(&builder.selected), 0);
        assert!(builder.selection_is_exact_active_root_partition());
        assert!(!builder.traversal_overflow);
    }

    #[test]
    fn blocked_large_exact_frontier_does_not_starve_a_fitting_exact_frontier() {
        let near_fine = TerrainPageKey::surface(0, 3, 3);
        let near_fine_parent = near_fine.parent().unwrap();
        let near_east = TerrainPageKey::surface(2, 1, 0);
        let near_north = TerrainPageKey::surface(2, 0, 1);
        let far_fine = TerrainPageKey::surface(0, 103, 0);
        let far_fine_parent = far_fine.parent().unwrap();
        let far_coarse = TerrainPageKey::surface(2, 26, 0);
        let far_children = far_coarse.refinement_children().unwrap();
        let mut capacity = VirtualTerrainCapacity::DEVELOPMENT_128_MIB;
        capacity.max_selected_pages = 8;
        let mut hierarchy = VirtualTerrainHierarchy::new(capacity).unwrap();
        for root in [
            near_fine_parent,
            near_east,
            near_north,
            far_fine_parent,
            far_coarse,
        ] {
            let (directory, pages) = surface_segment(root);
            hierarchy.register_region_directory(&directory).unwrap();
            for page in pages {
                hierarchy.install_page(page).unwrap();
            }
        }
        let selected = vec![near_fine, near_east, near_north, far_fine, far_coarse];
        let selected_owners = selected.iter().copied().map(|key| (key, key)).collect();
        let prior_refined = BTreeSet::new();
        let mut builder = cut_builder_for_owned_selection(
            &mut hierarchy,
            &prior_refined,
            default_exact_domain(),
            selected,
            selected_owners,
        );
        builder.view.force_exact_leaves = true;
        let [[minimum_x, minimum_z], [maximum_x, maximum_z]] =
            near_fine.horizontal_bounds().unwrap();
        builder.view.camera_position_metres = [
            f64::from(minimum_x + maximum_x) * 0.05,
            0.0,
            f64::from(minimum_z + maximum_z) * 0.05,
        ];

        builder.balance_surface_lod();

        assert!(builder.selected.contains(&near_east));
        assert!(builder.selected.contains(&near_north));
        assert!(!builder.selected.contains(&far_coarse));
        assert!(
            far_children
                .iter()
                .all(|child| builder.selected.contains(child)),
            "the later fitting exact component must progress after the nearer component fails"
        );
        assert_eq!(builder.next_balanced_refined, BTreeSet::from([far_coarse]));
        assert!(
            builder.selection_overflow,
            "only the finally unsatisfied exact frontier should report capacity overflow"
        );
    }

    #[test]
    fn two_sided_coarsening_closure_never_refines_its_temporary_owner() {
        let west_owner = TerrainPageKey::surface(1, 1, 0);
        let target = TerrainPageKey::surface(2, 1, 0);
        let east_owner = TerrainPageKey::surface(3, 1, 0);
        let west_leaves = west_owner.refinement_children().unwrap();
        let target_children = target.refinement_children().unwrap();
        let target_leaves = target_children
            .iter()
            .flat_map(|child| child.refinement_children().unwrap())
            .collect::<Vec<_>>();
        let mut hierarchy =
            VirtualTerrainHierarchy::new(VirtualTerrainCapacity::DEVELOPMENT_128_MIB).unwrap();
        let (east_directory, east_pages) = surface_segment(east_owner);
        hierarchy
            .register_region_directory(&east_directory)
            .unwrap();
        hierarchy
            .install_page(
                east_pages
                    .into_iter()
                    .find(|page| page.key == east_owner)
                    .unwrap(),
            )
            .unwrap();
        for key in [west_owner, target]
            .into_iter()
            .chain(west_leaves.iter().copied())
            .chain(target_children.iter().copied())
            .chain(target_leaves.iter().copied())
        {
            insert_unregistered_resident(&mut hierarchy, surface_page(key));
        }
        hierarchy.coherent_replacements.insert(target);
        let selected = west_leaves
            .iter()
            .copied()
            .chain(target_leaves.iter().copied())
            .chain([east_owner])
            .collect::<Vec<_>>();
        let selected_owners = west_leaves
            .iter()
            .copied()
            .map(|key| (key, west_owner))
            .chain(target_leaves.iter().copied().map(|key| (key, target)))
            .chain([(east_owner, east_owner)])
            .collect::<BTreeMap<_, _>>();
        let prior_refined = BTreeSet::new();
        let mut builder = cut_builder_for_owned_selection(
            &mut hierarchy,
            &prior_refined,
            default_exact_domain(),
            selected,
            selected_owners,
        );

        builder.balance_surface_lod();

        assert_eq!(
            builder.selected,
            vec![west_owner, target, east_owner],
            "coarsening must close over both target boundaries instead of oscillating"
        );
        assert_eq!(
            builder.next_balanced_selected,
            BTreeSet::from([west_owner, target])
        );
        assert!(
            !builder.next_balanced_refined.contains(&target),
            "a temporary owner cannot be refined again during the same balance solve"
        );
        assert!(
            builder
                .next_balanced_refined
                .is_disjoint(&builder.next_balanced_selected)
        );
        assert_eq!(exact_surface_lod_discontinuity_edges(&builder.selected), 0);
        assert!(builder.selection_is_exact_active_root_partition());
        assert!(!builder.traversal_overflow);
    }

    #[test]
    fn temporary_owner_is_stable_until_registered_blocker_arrives() {
        fn hierarchy_with_registration_order(
            reverse: bool,
        ) -> (VirtualTerrainHierarchy, Vec<TerrainPageV1>) {
            let fine_owner = TerrainPageKey::surface(1, 1, 0);
            let coarse_owner = TerrainPageKey::surface(2, 1, 0);
            let (fine_directory, fine_pages) = surface_segment(fine_owner);
            let (coarse_directory, coarse_pages) = surface_segment(coarse_owner);
            let mut hierarchy =
                VirtualTerrainHierarchy::new(VirtualTerrainCapacity::DEVELOPMENT_128_MIB).unwrap();
            let segments = if reverse {
                [
                    (&coarse_directory, &coarse_pages, coarse_owner),
                    (&fine_directory, &fine_pages, fine_owner),
                ]
            } else {
                [
                    (&fine_directory, &fine_pages, fine_owner),
                    (&coarse_directory, &coarse_pages, coarse_owner),
                ]
            };
            for (directory, pages, root) in segments {
                hierarchy.register_region_directory(directory).unwrap();
                for page in pages {
                    if root == fine_owner || page.key == root {
                        hierarchy.install_page(page.clone()).unwrap();
                    }
                }
            }
            (hierarchy, coarse_pages)
        }

        let fine_owner = TerrainPageKey::surface(1, 1, 0);
        let coarse_owner = TerrainPageKey::surface(2, 1, 0);
        let fine_children = fine_owner.refinement_children().unwrap();
        let coarse_children = coarse_owner.refinement_children().unwrap();
        let mut test_view = view(false);
        test_view.camera_position_metres = [50.0, 30.0, -200.0];
        test_view.camera_forward = [0.0, 0.0, 1.0];
        test_view.vertical_fov_radians = 2.5;
        test_view.aspect_ratio = 10.0;
        test_view.refine_above_pixels = 0.001;
        test_view.coarsen_below_pixels = 0.0;
        let (mut hierarchy, coarse_pages) = hierarchy_with_registration_order(false);
        let (mut reversed, _) = hierarchy_with_registration_order(true);

        let first = select_cut(&mut hierarchy, test_view).unwrap();
        let reversed_first = select_cut(&mut reversed, test_view).unwrap();
        assert_eq!(first.selected_pages, vec![fine_owner, coarse_owner]);
        assert_eq!(reversed_first.selected_pages, first.selected_pages);
        assert_eq!(reversed_first.requested_pages, first.requested_pages);
        assert!(first.is_renderable());
        assert_eq!(first.exact_surface_lod_discontinuities, 0);
        assert_eq!(
            hierarchy.balanced_selected_blockers.get(&fine_owner),
            Some(&BTreeSet::from([coarse_owner]))
        );

        let repeated = select_cut(&mut hierarchy, test_view).unwrap();
        assert_eq!(repeated.selected_pages, first.selected_pages);
        assert_eq!(repeated.fingerprint, first.fingerprint);
        assert_eq!(repeated.requested_pages, first.requested_pages);
        assert!(repeated.is_renderable());

        let (mut retired_neighbor, _) = hierarchy_with_registration_order(false);
        let blocked = select_cut(&mut retired_neighbor, test_view).unwrap();
        assert_eq!(blocked.selected_pages, first.selected_pages);
        retired_neighbor.set_active_roots([fine_owner]).unwrap();
        let unblocked = select_cut(&mut retired_neighbor, test_view).unwrap();
        assert!(
            fine_children
                .iter()
                .all(|key| unblocked.selected_pages.contains(key)),
            "retiring the blocker root must immediately release its temporary coarse neighbor"
        );
        assert!(!unblocked.selected_pages.contains(&coarse_owner));
        assert!(
            unblocked
                .requested_pages
                .iter()
                .all(|identity| identity.key.parent() != Some(coarse_owner)),
            "inactive blockers must not continue consuming stream requests"
        );
        assert!(retired_neighbor.balanced_selected_blockers.is_empty());
        assert!(unblocked.is_renderable());

        for page in coarse_pages
            .into_iter()
            .filter(|page| page.key != coarse_owner)
        {
            hierarchy.install_page(page).unwrap();
        }
        let resolved = select_cut(&mut hierarchy, test_view).unwrap();
        assert!(
            fine_children
                .iter()
                .all(|key| resolved.selected_pages.contains(key))
        );
        assert!(
            coarse_children
                .iter()
                .all(|key| resolved.selected_pages.contains(key))
        );
        assert!(resolved.requested_pages.is_empty());
        assert!(resolved.is_renderable());
        assert!(hierarchy.balanced_selected_blockers.is_empty());
        let resolved_repeated = select_cut(&mut hierarchy, test_view).unwrap();
        assert_eq!(resolved_repeated.selected_pages, resolved.selected_pages);
        assert_eq!(resolved_repeated.fingerprint, resolved.fingerprint);
    }

    #[test]
    fn mixed_level_active_roots_never_fallback_to_a_registered_inactive_ancestor() {
        let fine_owner = TerrainPageKey::surface(1, 3, 0);
        let inactive_ancestor = fine_owner.parent().unwrap();
        let coarse_owner = TerrainPageKey::surface(3, 1, 0);
        let coarse_children = coarse_owner.refinement_children().unwrap();
        let mut hierarchy =
            VirtualTerrainHierarchy::new(VirtualTerrainCapacity::DEVELOPMENT_128_MIB).unwrap();
        let (ancestor_directory, ancestor_pages) = surface_segment(inactive_ancestor);
        hierarchy
            .register_staging_directory(&ancestor_directory)
            .unwrap();
        for page in ancestor_pages {
            hierarchy.install_page(page).unwrap();
        }
        let (fine_directory, fine_pages) = surface_segment(fine_owner);
        hierarchy
            .register_refinement_directory(&fine_directory)
            .unwrap();
        for page in fine_pages.into_iter().filter(|page| page.key != fine_owner) {
            hierarchy.install_page(page).unwrap();
        }
        let (coarse_directory, coarse_pages) = surface_segment(coarse_owner);
        hierarchy
            .register_region_directory(&coarse_directory)
            .unwrap();
        hierarchy
            .install_page(
                coarse_pages
                    .into_iter()
                    .find(|page| page.key == coarse_owner)
                    .unwrap(),
            )
            .unwrap();
        hierarchy
            .set_active_roots([fine_owner, coarse_owner])
            .unwrap();
        let mut test_view = view(false);
        test_view.camera_position_metres = [128.0, 30.0, -200.0];
        test_view.camera_forward = [0.0, 0.0, 1.0];
        test_view.vertical_fov_radians = 2.5;
        test_view.aspect_ratio = 10.0;

        let cut = select_cut(&mut hierarchy, test_view).unwrap();

        assert_eq!(cut.selected_pages, vec![fine_owner, coarse_owner]);
        assert!(
            !cut.selected_pages.contains(&inactive_ancestor),
            "a resident inactive ancestor must never acquire terrain outside the visited root"
        );
        assert_eq!(
            cut.requested_pages
                .iter()
                .map(|identity| identity.key)
                .collect::<BTreeSet<_>>(),
            coarse_children.into_iter().collect()
        );
        assert!(cut.ownerless_roots.is_empty());
        assert!(!cut.selection_overflow);
        assert!(!cut.traversal_overflow);
        assert_eq!(cut.exact_surface_lod_discontinuities, 1);
        assert!(
            !cut.is_renderable(),
            "the otherwise valid active-root partition must fail closed only on its unresolved edge"
        );
    }

    fn surface_segment(root: TerrainPageKey) -> (TerrainHierarchyDirectoryV1, Vec<TerrainPageV1>) {
        let pages = root
            .refinement_children()
            .unwrap()
            .into_iter()
            .map(surface_page)
            .chain([surface_page(root)])
            .collect::<Vec<_>>();
        let directory =
            TerrainHierarchyDirectoryV1::from_surface_refinement_pages(root, &pages).unwrap();
        (directory, pages)
    }

    #[test]
    fn independently_streamed_surface_segment_refines_without_a_second_owner() {
        let root = TerrainPageKey::surface(2, -1, -1);
        let (base_directory, base_pages) = surface_segment(root);
        let mut orphan =
            VirtualTerrainHierarchy::new(VirtualTerrainCapacity::DEVELOPMENT_128_MIB).unwrap();
        assert_eq!(
            orphan.register_refinement_directory(&base_directory),
            Err(VirtualTerrainError::InvalidDirectory)
        );
        let mut hierarchy =
            VirtualTerrainHierarchy::new(VirtualTerrainCapacity::DEVELOPMENT_128_MIB).unwrap();
        hierarchy
            .register_region_directory(&base_directory)
            .unwrap();
        hierarchy
            .install_page(
                base_pages
                    .iter()
                    .find(|page| page.key == root)
                    .unwrap()
                    .clone(),
            )
            .unwrap();

        let initial = select_cut(&mut hierarchy, view(true)).unwrap();
        assert_eq!(initial.selected_pages, vec![root]);
        assert_eq!(initial.requested_pages.len(), 4);
        assert!(initial.refinement_roots.is_empty());
        assert!(!initial.covers_exact_surface_domain(&exact_domain(view(true), 10.0)));

        for child in base_pages.iter().filter(|page| page.key != root) {
            hierarchy.install_page(child.clone()).unwrap();
        }
        let children = root.refinement_children().unwrap();
        let terminal = select_cut(&mut hierarchy, view(true)).unwrap();
        assert_eq!(
            terminal
                .selected_pages
                .iter()
                .copied()
                .collect::<BTreeSet<_>>(),
            children.iter().copied().collect()
        );
        assert_eq!(
            terminal
                .refinement_roots
                .iter()
                .copied()
                .collect::<BTreeSet<_>>(),
            children.iter().copied().collect()
        );

        let refined_root = children[0];
        let (refinement_directory, refinement_pages) = surface_segment(refined_root);
        hierarchy
            .register_refinement_directory(&refinement_directory)
            .unwrap();
        assert_eq!(hierarchy.registered_roots().collect::<Vec<_>>(), vec![root]);
        assert!(
            hierarchy
                .directory_node(refined_root)
                .is_some_and(|node| node.has_children && !node.is_root)
        );
        for child in refinement_pages
            .iter()
            .filter(|page| page.key != refined_root)
        {
            hierarchy.install_page(child.clone()).unwrap();
        }

        let refined = select_cut(&mut hierarchy, view(true)).unwrap();
        assert!(!refined.selected_pages.contains(&refined_root));
        assert_eq!(refined.selected_pages.len(), 7);
        assert_eq!(
            refined
                .selected_pages
                .iter()
                .filter(|key| key.parent() == Some(refined_root))
                .count(),
            4
        );
        assert_eq!(
            refined
                .refinement_roots
                .iter()
                .copied()
                .collect::<BTreeSet<_>>(),
            children.iter().copied().skip(1).collect()
        );

        let removed = hierarchy
            .remove_region_directory(root)
            .into_iter()
            .collect::<BTreeSet<_>>();
        assert!(removed.contains(&root));
        assert!(removed.contains(&refined_root));
        assert!(hierarchy.nodes().next().is_none());
        assert!(hierarchy.roots().next().is_none());
    }

    #[test]
    fn staged_child_roots_atomically_replace_their_registered_parent() {
        let pages = hierarchy_pages();
        let parent = pages
            .iter()
            .find(|page| page.key.level == 1)
            .unwrap()
            .clone();
        let children = pages
            .iter()
            .filter(|page| page.key.level == 0)
            .cloned()
            .collect::<Vec<_>>();
        let directory = TerrainHierarchyDirectoryV1::from_pages(&pages).unwrap();
        let mut hierarchy =
            VirtualTerrainHierarchy::new(VirtualTerrainCapacity::DEVELOPMENT_128_MIB).unwrap();
        hierarchy.register_region_directory(&directory).unwrap();
        for page in &pages {
            hierarchy.install_page(page.clone()).unwrap();
        }

        assert_eq!(hierarchy.roots().collect::<Vec<_>>(), vec![parent.key]);
        assert_eq!(hierarchy.registered_roots().count(), 1);

        hierarchy
            .set_active_roots(children.iter().map(|page| page.key))
            .unwrap();
        assert_eq!(
            hierarchy.roots().collect::<BTreeSet<_>>(),
            children.iter().map(|page| page.key).collect()
        );
        assert!(hierarchy.registered_roots().any(|key| key == parent.key));
    }

    #[test]
    fn active_root_cut_rejects_unknown_and_overlapping_owners() {
        let pages = hierarchy_pages();
        let parent = pages
            .iter()
            .find(|page| page.key.level == 1)
            .unwrap()
            .clone();
        let child = pages
            .iter()
            .find(|page| page.key.level == 0)
            .unwrap()
            .clone();
        let directory = TerrainHierarchyDirectoryV1::from_pages(&pages).unwrap();
        let mut hierarchy =
            VirtualTerrainHierarchy::new(VirtualTerrainCapacity::DEVELOPMENT_128_MIB).unwrap();
        hierarchy.register_staging_directory(&directory).unwrap();

        assert!(matches!(
            hierarchy.set_active_roots([parent.key, child.key]),
            Err(VirtualTerrainError::OverlappingRoots(_, _))
        ));
        let unknown = TerrainPageKey {
            level: 0,
            coord: [999, 999, 999],
        };
        assert_eq!(
            hierarchy.set_active_roots([unknown]),
            Err(VirtualTerrainError::UnknownRoot(unknown))
        );
        assert!(hierarchy.roots().next().is_none());
    }

    #[test]
    fn active_root_cut_rejects_partial_child_coverage() {
        let (mut hierarchy, pages) = hierarchy();
        for page in &pages {
            hierarchy.install_page(page.clone()).unwrap();
        }
        let child = pages.iter().find(|page| page.key.level == 0).unwrap().key;
        let parent = child.parent().unwrap();
        assert_eq!(
            hierarchy.set_active_roots([child]),
            Err(VirtualTerrainError::IncompleteRootReplacement(parent))
        );
        assert_eq!(hierarchy.roots().collect::<Vec<_>>(), vec![parent]);
    }

    #[test]
    fn incomplete_refinement_keeps_parent_and_requests_the_whole_missing_group() {
        let (mut hierarchy, pages) = hierarchy();
        let root = pages.iter().find(|page| page.key.level == 1).unwrap();
        hierarchy.install_page(root.clone()).unwrap();
        let cut = select_cut(&mut hierarchy, view(true)).unwrap();
        assert!(cut.is_renderable());
        assert_eq!(cut.selected_pages, vec![root.key]);
        assert_eq!(cut.requested_pages.len(), TERRAIN_PAGE_MAX_CHILDREN);
        assert!(cut.ownerless_roots.is_empty());
        assert!(!cut.covers_exact_surface_domain(&exact_domain(view(true), 1.0)));
    }

    #[test]
    fn complete_group_atomically_replaces_parent_without_overlap() {
        let (mut hierarchy, pages) = hierarchy();
        for page in pages {
            hierarchy.install_page(page).unwrap();
        }
        let cut = select_cut(&mut hierarchy, view(true)).unwrap();
        assert!(cut.is_renderable());
        assert_eq!(cut.selected_pages.len(), TERRAIN_PAGE_MAX_CHILDREN);
        assert!(cut.selected_pages.iter().all(|key| key.level == 0));
        assert!(cut.requested_pages.is_empty());
        assert_eq!(
            cut.fingerprint,
            hierarchy.selected_fingerprint(&cut.selected_pages)
        );
        for (index, left) in cut.selected_pages.iter().enumerate() {
            for right in &cut.selected_pages[index + 1..] {
                assert_ne!(left.ancestor_at(1), Some(*right));
                assert_ne!(right.ancestor_at(1), Some(*left));
            }
        }
    }

    #[test]
    fn complete_group_keeps_a_full_root_partition_at_the_frustum_edge() {
        let (mut hierarchy, pages) = hierarchy();
        for page in pages {
            hierarchy.install_page(page).unwrap();
        }
        let mut edge_view = view(true);
        edge_view.camera_position_metres = [-3.2, 3.2, -6.3];
        edge_view.camera_forward = [0.0, 0.0, -1.0];
        let cut = select_cut(&mut hierarchy, edge_view).unwrap();
        assert!(cut.is_renderable());
        assert_eq!(cut.selected_pages.len(), TERRAIN_PAGE_MAX_CHILDREN);
        assert!(cut.selected_pages.iter().all(|key| key.level == 0));
    }

    #[test]
    fn exact_parent_stays_selected_without_forced_leaf_refinement() {
        let (mut hierarchy, pages) = hierarchy();
        for page in pages {
            hierarchy.install_page(page).unwrap();
        }
        let cut = select_cut(&mut hierarchy, view(false)).unwrap();
        assert_eq!(cut.selected_pages.len(), 1);
        assert_eq!(cut.selected_pages[0].level, 1);
        assert_eq!(cut.selected_primitives, 1);
    }

    #[test]
    fn optional_distant_refinement_capacity_keeps_a_renderable_coarse_cut() {
        let root = TerrainPageKey::surface(2, 10, 0);
        let (directory, pages) = surface_segment(root);
        let mut capacity = VirtualTerrainCapacity::DEVELOPMENT_128_MIB;
        capacity.max_selected_pages = 1;
        let mut hierarchy = VirtualTerrainHierarchy::new(capacity).unwrap();
        hierarchy.register_region_directory(&directory).unwrap();
        for page in pages {
            hierarchy.install_page(page).unwrap();
        }
        let mut distant_view = view(false);
        distant_view.camera_position_metres = [0.0, 0.0, 6.4];
        distant_view.camera_forward = [1.0, 0.0, 0.0];

        let cut = select_cut(&mut hierarchy, distant_view).unwrap();

        assert_eq!(cut.selected_pages, vec![root]);
        assert!(
            projected_page_error_pixels(&hierarchy.directory_node(root).unwrap(), distant_view)
                > distant_view.refine_above_pixels,
            "the fixture must want optional projected-error refinement"
        );
        assert!(!cut.selection_overflow);
        assert!(cut.is_renderable());
    }

    #[test]
    fn exact_surface_domain_selects_leaf_lattice_without_debug_override() {
        let root = TerrainPageKey::surface(1, -1, -1);
        let (directory, pages) = surface_segment(root);
        let mut hierarchy =
            VirtualTerrainHierarchy::new(VirtualTerrainCapacity::DEVELOPMENT_128_MIB).unwrap();
        hierarchy.register_region_directory(&directory).unwrap();
        for page in pages {
            hierarchy.install_page(page).unwrap();
        }
        let mut exact_view = view(false);
        exact_view.camera_position_metres = [-3.2, 3.2, -3.2];
        let exact_domain = exact_domain(exact_view, 3.0);
        let cut = hierarchy.select_cut(exact_view, &exact_domain).unwrap();
        assert_eq!(cut.selected_pages.len(), 4);
        assert!(cut.selected_pages.iter().all(|key| key.level == 0));
        assert!(cut.requested_pages.is_empty());
        assert!(cut.covers_exact_surface_domain(&exact_domain));
    }

    #[test]
    fn stationary_exact_surface_domain_is_the_single_half_open_owner() {
        let domain = ExactSurfaceDomain::swept_horizontal_capsule(
            [0.25, 7.0, 0.5],
            [0.25, 7.0, 0.5],
            0.0,
            16,
        );

        assert!(domain.is_complete());
        assert_eq!(
            domain.required_leaves().collect::<Vec<_>>(),
            vec![TerrainPageKey::surface(0, 0, 0)]
        );
        assert_eq!(
            domain.required_pages_at_level(1).collect::<Vec<_>>(),
            vec![TerrainPageKey::surface(1, 0, 0)]
        );
    }

    #[test]
    fn exact_surface_domain_floors_negative_coordinates_without_aliasing_zero() {
        let just_negative = ExactSurfaceDomain::swept_horizontal_capsule(
            [-0.01, 0.0, -0.01],
            [-0.01, 0.0, -0.01],
            0.0,
            16,
        );
        let negative_boundary = ExactSurfaceDomain::swept_horizontal_capsule(
            [-3.2, 0.0, -3.2],
            [-3.2, 0.0, -3.2],
            0.0,
            16,
        );

        assert_eq!(
            just_negative.required_leaves().collect::<Vec<_>>(),
            vec![TerrainPageKey::surface(0, -1, -1)]
        );
        assert_eq!(
            negative_boundary.required_leaves().collect::<Vec<_>>(),
            vec![TerrainPageKey::surface(0, -1, -1)],
            "an exact negative page boundary belongs to the half-open page on its positive side"
        );
    }

    #[test]
    fn exact_surface_domain_boundary_contact_uses_half_open_point_ownership() {
        let stationary =
            ExactSurfaceDomain::swept_horizontal_capsule([3.2, 0.0, 0.5], [3.2, 0.0, 0.5], 0.0, 16);
        let arriving =
            ExactSurfaceDomain::swept_horizontal_capsule([0.5, 0.0, 0.5], [3.2, 0.0, 0.5], 0.0, 16);
        let tangent =
            ExactSurfaceDomain::swept_horizontal_capsule([1.6, 0.0, 1.6], [1.6, 0.0, 1.6], 1.6, 16);

        assert_eq!(
            stationary.required_leaves().collect::<Vec<_>>(),
            vec![TerrainPageKey::surface(0, 1, 0)]
        );
        assert_eq!(
            arriving.required_leaves().collect::<Vec<_>>(),
            vec![
                TerrainPageKey::surface(0, 0, 0),
                TerrainPageKey::surface(0, 1, 0),
            ],
            "the previous owner contains the segment interior while the endpoint belongs to the next page"
        );
        assert_eq!(
            tangent.required_leaves().collect::<BTreeSet<_>>(),
            BTreeSet::from([
                TerrainPageKey::surface(0, 0, 0),
                TerrainPageKey::surface(0, 0, 1),
                TerrainPageKey::surface(0, 1, 0),
            ]),
            "a closed capsule includes its positive-side tangent owners, but not a diagonal page beyond its radius"
        );
    }

    #[test]
    fn exact_surface_domain_enumerates_every_leaf_on_a_fast_diagonal() {
        let forward = ExactSurfaceDomain::swept_horizontal_capsule(
            [0.1, 0.0, 0.2],
            [12.7, 0.0, 9.5],
            0.0,
            64,
        );
        let reverse = ExactSurfaceDomain::swept_horizontal_capsule(
            [12.7, 0.0, 9.5],
            [0.1, 0.0, 0.2],
            0.0,
            64,
        );
        let expected = BTreeSet::from([
            TerrainPageKey::surface(0, 0, 0),
            TerrainPageKey::surface(0, 1, 0),
            TerrainPageKey::surface(0, 1, 1),
            TerrainPageKey::surface(0, 2, 1),
            TerrainPageKey::surface(0, 2, 2),
            TerrainPageKey::surface(0, 3, 2),
        ]);

        assert!(forward.is_complete());
        assert_eq!(forward.required_leaves().collect::<BTreeSet<_>>(), expected);
        assert_eq!(
            reverse.required_leaves().collect::<BTreeSet<_>>(),
            forward.required_leaves().collect::<BTreeSet<_>>(),
            "prediction enumeration must not depend on travel direction"
        );
        assert_eq!(
            reverse.prediction_fingerprint(),
            forward.prediction_fingerprint()
        );
    }

    #[test]
    fn exact_surface_domain_fails_closed_when_the_stationary_core_is_invalid() {
        let coordinate_overflow = ExactSurfaceDomain::swept_horizontal_capsule(
            [1.0e30, 0.0, 0.0],
            [1.0e30, 0.0, 0.0],
            0.0,
            16,
        );
        let invalid = ExactSurfaceDomain::swept_horizontal_capsule(
            [f64::NAN, 0.0, 0.0],
            [0.0, 0.0, 0.0],
            0.0,
            16,
        );
        let nominal = cut_with_selected(vec![TerrainPageKey::surface(0, 0, 0)]);

        for domain in [coordinate_overflow, invalid] {
            assert!(!domain.core_is_complete());
            assert!(!domain.is_complete());
            assert_eq!(domain.required_leaf_count(), 0);
            assert!(!nominal.covers_exact_surface_domain(&domain));
        }
    }

    #[test]
    fn exact_surface_domain_keeps_a_nonempty_core_when_prediction_overflows() {
        let domain =
            ExactSurfaceDomain::swept_horizontal_capsule([0.1, 0.0, 0.1], [12.7, 0.0, 0.1], 0.0, 3);
        let nominal = cut_with_selected(vec![TerrainPageKey::surface(0, 0, 0)]);

        assert!(domain.core_is_complete());
        assert!(!domain.prediction_is_complete());
        assert!(!domain.is_complete());
        assert_eq!(domain.core_required_leaf_count(), 1);
        assert_eq!(domain.prediction_required_leaf_count(), 0);
        assert_eq!(domain.required_leaf_count(), 1);
        assert_ne!(domain.fingerprint(), 0);
        assert_eq!(domain.prediction_fingerprint(), 0);
        assert!(nominal.covers_exact_surface_domain(&domain));
    }

    #[test]
    fn exact_surface_domain_cache_reuses_only_identical_continuous_inputs() {
        let mut cache = ExactSurfaceDomainCache::default();
        let first = cache.resolve([0.1, 0.0, 0.1], [3.3, 0.0, 0.1], 0.0, 16);
        let identical = cache.resolve([0.1, 0.0, 0.1], [3.3, 0.0, 0.1], 0.0, 16);
        let moved_within_the_same_leaf = cache.resolve([0.2, 0.0, 0.1], [3.3, 0.0, 0.1], 0.0, 16);

        assert_eq!(cache.rebuilds(), 2);
        assert!(first.shares_storage_with(&identical));
        assert!(!first.shares_storage_with(&moved_within_the_same_leaf));
    }

    #[test]
    fn bounded_exact_prediction_can_span_seventeen_coverage_columns() {
        const MAX_LEAVES: usize = 16_384;
        let leaf_span = f64::from(TERRAIN_PAGE_EDGE_SAMPLES) * 0.1;
        let first_leaf_x = (1_i32 << TERRAIN_COVERAGE_ROOT_LEVEL) - 1;
        let last_leaf_x = first_leaf_x + i32::try_from(MAX_LEAVES).unwrap() - 1;
        let domain = ExactSurfaceDomain::swept_horizontal_capsule(
            [f64::from(first_leaf_x) * leaf_span + 0.1, 0.0, 0.1],
            [f64::from(last_leaf_x) * leaf_span + 0.1, 0.0, 0.1],
            0.0,
            MAX_LEAVES,
        );

        assert!(domain.is_complete());
        assert_eq!(domain.required_leaf_count(), MAX_LEAVES);
        assert_eq!(
            domain
                .required_pages_at_level(TERRAIN_COVERAGE_ROOT_LEVEL)
                .count(),
            17,
            "an exact proof domain is not bounded by the ordinary 16-column quality prefix"
        );
    }

    #[test]
    fn exact_surface_coverage_fails_closed_for_an_unregistered_gap() {
        let domain =
            ExactSurfaceDomain::swept_horizontal_capsule([0.5, 0.0, 0.5], [3.5, 0.0, 0.5], 0.0, 16);
        let gap = cut_with_selected(vec![
            TerrainPageKey::surface(0, 0, 0),
            TerrainPageKey::surface(1, 0, 0),
        ]);

        assert_eq!(domain.required_leaf_count(), 2);
        assert_eq!(gap.exact_surface_coverage(&domain), 1);
        assert!(
            !gap.covers_exact_surface_domain(&domain),
            "a coarse or absent owner cannot certify an unregistered exact leaf"
        );
    }

    #[test]
    fn exact_surface_promotion_rejects_an_equal_count_leaf_swap() {
        let domain =
            ExactSurfaceDomain::swept_horizontal_capsule([0.5, 0.0, 0.5], [3.5, 0.0, 0.5], 0.0, 16);
        let committed = cut_with_selected(vec![TerrainPageKey::surface(0, 0, 0)]);
        let candidate = cut_with_selected(vec![TerrainPageKey::surface(0, 1, 0)]);

        assert_eq!(committed.exact_surface_coverage(&domain), 1);
        assert_eq!(candidate.exact_surface_coverage(&domain), 1);
        assert!(
            !candidate.preserves_exact_surface_coverage(&committed, &domain),
            "equal scalar coverage must not hide the loss of a previously exact leaf"
        );
    }

    #[test]
    fn exact_surface_domain_rejects_a_coarse_owner_crossed_between_endpoints() {
        let current = TerrainPageKey::surface(0, 0, 0);
        let coarse_ahead = TerrainPageKey::surface(1, 1, 0);
        let cut = cut_with_selected(vec![current, coarse_ahead]);
        let short =
            ExactSurfaceDomain::swept_horizontal_capsule([0.5, 3.0, 0.5], [2.5, 3.0, 0.5], 0.0, 16);
        let long = ExactSurfaceDomain::swept_horizontal_capsule(
            [0.5, 3.0, 0.5],
            [10.0, 3.0, 0.5],
            0.0,
            16,
        );
        assert!(cut.covers_exact_surface_domain(&short));
        assert!(
            !cut.covers_exact_surface_domain(&long),
            "a coarse page crossed only in the middle of motion must revoke virtual ownership"
        );

        let exact = cut_with_selected((0..=3).map(|x| TerrainPageKey::surface(0, x, 0)).collect());
        assert!(exact.covers_exact_surface_domain(&long));
    }

    #[test]
    fn segment_aabb_distance_detects_corner_crossings_and_radius() {
        let minimum = [0.0, 0.0];
        let maximum = [1.0, 1.0];
        assert_eq!(
            segment_aabb_distance_squared_2d([-1.0, -1.0], [2.0, 2.0], minimum, maximum),
            0.0
        );
        assert_eq!(
            segment_aabb_distance_squared_2d([-1.0, 0.5], [-0.25, 0.5], minimum, maximum),
            0.25 * 0.25
        );
        assert_eq!(
            segment_aabb_distance_squared_2d([-1.0, -1.0], [-0.5, -0.5], minimum, maximum),
            0.5
        );
    }

    #[test]
    fn missing_root_is_explicitly_ownerless_and_requested() {
        let (mut hierarchy, _) = hierarchy();
        let cut = select_cut(&mut hierarchy, view(false)).unwrap();
        assert!(!cut.is_renderable());
        assert_eq!(cut.ownerless_roots.len(), 1);
        assert_eq!(cut.requested_pages.len(), 1);
    }

    #[test]
    fn empty_hierarchy_is_not_a_renderable_owner() {
        let mut hierarchy =
            VirtualTerrainHierarchy::new(VirtualTerrainCapacity::DEVELOPMENT_128_MIB).unwrap();
        let cut = select_cut(&mut hierarchy, view(false)).unwrap();
        assert!(cut.selected_pages.is_empty());
        assert!(cut.ownerless_roots.is_empty());
        assert!(!cut.is_renderable());
    }

    #[test]
    fn removing_a_region_reclaims_every_page_node_and_accounted_byte() {
        let (mut hierarchy, pages) = hierarchy();
        let root = pages.iter().find(|page| page.key.level == 1).unwrap().key;
        for page in pages {
            hierarchy.install_page(page).unwrap();
        }
        assert_eq!(hierarchy.resident_usage().0, 9);
        let removed = hierarchy.remove_region_directory(root);
        assert_eq!(removed.len(), 9);
        assert_eq!(hierarchy.resident_usage(), (0, 0, 0));
        assert_eq!(hierarchy.nodes().count(), 0);
        assert_eq!(hierarchy.roots().count(), 0);
        assert_eq!(hierarchy.source_identity_hash(), None);
        assert!(hierarchy.remove_region_directory(root).is_empty());
    }

    #[test]
    fn invalid_capacity_and_view_fail_closed() {
        let mut capacity = VirtualTerrainCapacity::DEVELOPMENT_128_MIB;
        capacity.max_feedback_pages = 0;
        assert!(matches!(
            VirtualTerrainHierarchy::new(capacity),
            Err(VirtualTerrainError::InvalidCapacity)
        ));
        let (mut hierarchy, _) = hierarchy();
        let mut invalid = view(false);
        invalid.aspect_ratio = f64::NAN;
        assert_eq!(
            hierarchy.select_cut(invalid, default_exact_domain()),
            Err(VirtualTerrainError::InvalidView)
        );
    }

    #[test]
    fn page_distance_and_projection_use_canonical_ten_centimetre_scale() {
        let key = TerrainPageKey {
            level: 0,
            coord: [0, 0, 0],
        };
        let bounds = key.bounds().expect("page bounds");
        assert_eq!(distance_to_page_metres(bounds, [0.0, 0.0, 6.4]), 3.2);
        let node = TerrainHierarchyNode {
            key,
            bounds,
            revision: 1,
            content_fingerprint: [1; 32],
            errors: TerrainErrorBounds {
                geometric_millivoxels: 1_000,
                ..TerrainErrorBounds::EXACT
            },
            topology: voxels_world::TerrainTopologyClass::SingleRunColumns,
            representation: voxels_world::TerrainPageRepresentationKind::SurfaceCluster,
            encoded_bytes: 1,
            source_geometry_bytes: 24,
            has_children: false,
            is_root: true,
        };
        let mut projection_view = view(false);
        projection_view.camera_position_metres = [1.6, 1.6, 6.4];
        let pixels = projected_page_error_pixels(&node, projection_view);
        assert!(pixels > 30.0 && pixels < 32.0);
    }

    #[test]
    fn negative_region_root_is_conservatively_visible() {
        let visible = page_is_visible(
            TerrainPageKey {
                level: 1,
                coord: [-1, 0, -1],
            }
            .bounds()
            .expect("page bounds"),
            view(false),
        );
        assert!(visible);
        let _ = VoxelCoord::new(0, 0, 0);
    }
}
