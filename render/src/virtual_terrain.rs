//! Bounded resident hierarchy and one-owner cut selection for virtual microvoxel terrain.
//!
//! This module is deliberately independent of WGPU resources. It is the CPU oracle for the GPU
//! traversal: roots remain valid fallbacks, refinement replaces a parent only as a complete
//! certified child group, and request feedback is bounded without ever manufacturing geometry.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use voxels_world::{
    TERRAIN_PAGE_EDGE_SAMPLES, TERRAIN_PAGE_MAX_CHILDREN, TerrainHierarchyDirectoryV1,
    TerrainHierarchyNode, TerrainPageKey, TerrainPageRepresentation, TerrainPageTransferIdentity,
    TerrainPageV1, WorldSourceIdentityHash, encode_terrain_page, reconstruct_exact_terrain_surface,
    validate_terrain_replacement,
};

const FINGERPRINT_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FINGERPRINT_PRIME: u64 = 0x0000_0100_0000_01b3;
const NORMAL_ERROR_PIXELS_PER_RADIAN: f64 = 0.25;

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
    /// Surface pages intersecting this horizontal radius must resolve to the 10 cm leaf lattice.
    pub exact_surface_radius_metres: f64,
    /// Reference/debug override. Production selection normally follows certified error.
    pub force_exact_leaves: bool,
}

impl VirtualTerrainView {
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
            && self.exact_surface_radius_metres.is_finite()
            && self.exact_surface_radius_metres >= 0.0
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

    /// Returns whether every selected surface owner intersecting the player's required vicinity
    /// is an exact 10 cm leaf.
    ///
    /// Renderability alone proves only that a cut is a complete partition. A complete coarse cut
    /// is a valid distant fallback, but publishing it underneath the player creates giant
    /// polygons and repeated coarse/detail oscillation.
    pub fn has_exact_surface_vicinity(
        &self,
        camera_position_metres: [f64; 3],
        radius_metres: f64,
    ) -> bool {
        if !camera_position_metres.into_iter().all(f64::is_finite)
            || !radius_metres.is_finite()
            || radius_metres < 0.0
        {
            return false;
        }
        let mut intersecting_pages = 0usize;
        for key in self.selected_pages.iter().filter(|key| key.is_surface()) {
            let Some([minimum, maximum]) = key.horizontal_bounds() else {
                return false;
            };
            let distance_squared = [0, 1]
                .into_iter()
                .map(|axis| {
                    let point = camera_position_metres[axis * 2];
                    let minimum = f64::from(minimum[axis]) * 0.1;
                    let maximum = f64::from(maximum[axis]) * 0.1;
                    let distance = if point < minimum {
                        minimum - point
                    } else if point > maximum {
                        point - maximum
                    } else {
                        0.0
                    };
                    distance * distance
                })
                .sum::<f64>();
            if distance_squared > radius_metres * radius_metres {
                continue;
            }
            intersecting_pages += 1;
            if key.level != 0 {
                return false;
            }
        }
        intersecting_pages > 0
    }
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
        cut_fingerprint(selected, self)
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
        }
        if self.directory_fingerprints.is_empty() {
            self.source_identity_hash = None;
        }
        key_set.into_iter().collect()
    }

    pub fn select_cut(
        &mut self,
        view: VirtualTerrainView,
    ) -> Result<VirtualTerrainCut, VirtualTerrainError> {
        if !view.validates() {
            return Err(VirtualTerrainError::InvalidView);
        }
        self.frame = self.frame.wrapping_add(1).max(1);
        let frame = self.frame;
        let prior_refined = self.refined_last_cut.clone();
        let mut builder = CutBuilder {
            hierarchy: self,
            view,
            frame,
            prior_refined: &prior_refined,
            next_refined: BTreeSet::new(),
            selected: Vec::new(),
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
                        || (key.is_surface()
                            && page_intersects_exact_surface_radius(node.bounds, view))
                })
            })
            .collect::<Vec<_>>();
        for root in roots {
            if builder.selected.len() >= builder.hierarchy.capacity.max_selected_pages {
                builder.selection_overflow = true;
                builder.ownerless_roots.push(root);
                builder.request(root);
                continue;
            }
            builder.visit(root, true);
        }
        builder.selected.sort_unstable();
        builder.ownerless_roots.sort_unstable();
        let exact_surface_lod_discontinuities =
            exact_surface_lod_discontinuity_edges(&builder.selected);
        let mut requested_pages = builder.requests.into_iter().collect::<Vec<_>>();
        requested_pages.sort_unstable_by_key(|identity| identity.key);
        let refinement_roots = builder.refinement_requests.into_iter().collect();
        let fingerprint = cut_fingerprint(&builder.selected, builder.hierarchy);
        builder.hierarchy.refined_last_cut = builder.next_refined;
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

/// Counts fine edge segments whose selected neighbor skips an intermediate surface level.
///
/// Looking outward from the finer page makes the audit bounded by four directions times the
/// hierarchy depth rather than comparing every selected page with every other page.
fn exact_surface_lod_discontinuity_edges(selected: &[TerrainPageKey]) -> usize {
    let selected = selected.iter().copied().collect::<BTreeSet<_>>();
    let maximum_level = selected
        .iter()
        .filter(|key| key.is_surface())
        .map(|key| key.level)
        .max()
        .unwrap_or(0);
    selected
        .iter()
        .copied()
        // Levels 1 and 2 are the transition rings around the exact level-0 lattice. Letting either
        // ring skip a level merely moves the same sharp step a few metres outward where a moving
        // player can immediately catch it.
        .filter(|key| key.is_surface() && key.level <= 2)
        .map(|key| {
            [
                [key.coord[0].saturating_sub(1), key.coord[2]],
                [key.coord[0].saturating_add(1), key.coord[2]],
                [key.coord[0], key.coord[2].saturating_sub(1)],
                [key.coord[0], key.coord[2].saturating_add(1)],
            ]
            .into_iter()
            .filter(|neighbor| {
                let same_level = TerrainPageKey::surface(key.level, neighbor[0], neighbor[1]);
                ((key.level.saturating_add(2))..=maximum_level).any(|level| {
                    same_level
                        .ancestor_at(level)
                        .is_some_and(|ancestor| selected.contains(&ancestor))
                })
            })
            .count()
        })
        .sum()
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
    frame: u64,
    prior_refined: &'a BTreeSet<TerrainPageKey>,
    next_refined: BTreeSet<TerrainPageKey>,
    selected: Vec<TerrainPageKey>,
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
    fn visit(&mut self, key: TerrainPageKey, root: bool) {
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
        let exact_surface =
            key.is_surface() && page_intersects_exact_surface_radius(node.bounds, self.view);
        let graded_surface = key.is_surface()
            && page_intersects_surface_lod_guard(node.bounds, key.level, self.view);
        if root && !page_is_visible(node.bounds, self.view) && !exact_surface {
            return;
        }
        if self.visited_nodes >= self.hierarchy.capacity.max_traversal_nodes {
            self.traversal_overflow = true;
            if root {
                self.ownerless_roots.push(key);
            } else {
                self.select(key);
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
        let threshold = if self.prior_refined.contains(&key) {
            self.view.coarsen_below_pixels
        } else {
            self.view.refine_above_pixels
        };
        let projected_error = projected_page_error_pixels(&node, self.view);
        let wants_more_detail = self.view.force_exact_leaves
            || exact_surface
            || graded_surface
            || projected_error > threshold;
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
                self.select(key);
                return;
            };
            if self.hierarchy.replacement_is_resident_and_coherent(key) {
                self.next_refined.insert(key);
                for child in children {
                    self.visit(child, false);
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
        } else if wants_refinement {
            self.selection_overflow = true;
        }
        self.select(key);
    }

    fn select(&mut self, key: TerrainPageKey) {
        let Some(resident) = self.hierarchy.resident.get_mut(&key) else {
            return;
        };
        if self.selected.len() >= self.hierarchy.capacity.max_selected_pages {
            self.selection_overflow = true;
            return;
        }
        resident.last_selected_frame = self.frame;
        self.selected.push(key);
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

fn page_intersects_exact_surface_radius(
    bounds: voxels_world::VoxelBounds,
    view: VirtualTerrainView,
) -> bool {
    page_horizontal_distance_squared(bounds, view)
        <= view.exact_surface_radius_metres * view.exact_surface_radius_metres
}

/// Grades the surface quadtree outward one spatial page at a time.
///
/// Without this guard, an exact level-0 page can share an edge directly with a level-3 or
/// coarser page even though all intermediate directories are resident. Besides looking like a
/// giant block beside 10 cm terrain, that discontinuity makes any conforming boundary needlessly
/// large. Each hierarchy level gets one page-width guard band, so adjacent selected pages can
/// converge incrementally instead of jumping several levels at once.
fn page_intersects_surface_lod_guard(
    bounds: voxels_world::VoxelBounds,
    level: u8,
    view: VirtualTerrainView,
) -> bool {
    if level == 0 {
        return false;
    }
    let page_width_metres = f64::from(bounds.max.x.saturating_sub(bounds.min.x)) * 0.1;
    // Two page widths leave a complete intermediate ring even at a diagonal corner, where the
    // Euclidean distance across one square alone is not enough to prevent a two-level jump.
    let radius = view.exact_surface_radius_metres + page_width_metres * 2.0;
    page_horizontal_distance_squared(bounds, view) <= radius * radius
}

fn page_horizontal_distance_squared(
    bounds: voxels_world::VoxelBounds,
    view: VirtualTerrainView,
) -> f64 {
    let minimum = bounds.min.as_array().map(|value| f64::from(value) * 0.1);
    let maximum = bounds.max.as_array().map(|value| f64::from(value) * 0.1);
    [0, 2]
        .into_iter()
        .map(|axis| {
            let distance = if view.camera_position_metres[axis] < minimum[axis] {
                minimum[axis] - view.camera_position_metres[axis]
            } else if view.camera_position_metres[axis] > maximum[axis] {
                view.camera_position_metres[axis] - maximum[axis]
            } else {
                0.0
            };
            distance * distance
        })
        .sum()
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
            exact_surface_radius_metres: 0.0,
            force_exact_leaves,
        }
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
        let error = 1_000_u32 << u32::from(key.level);
        build_sampled_heightfield_terrain_page(
            identity(),
            key,
            7,
            &samples,
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

        let initial = hierarchy.select_cut(view(true)).unwrap();
        assert_eq!(initial.selected_pages, vec![root]);
        assert_eq!(initial.requested_pages.len(), 4);
        assert!(initial.refinement_roots.is_empty());
        assert!(!initial.has_exact_surface_vicinity([-3.2, 3.2, 8.0], 10.0));

        for child in base_pages.iter().filter(|page| page.key != root) {
            hierarchy.install_page(child.clone()).unwrap();
        }
        let children = root.refinement_children().unwrap();
        let terminal = hierarchy.select_cut(view(true)).unwrap();
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

        let refined = hierarchy.select_cut(view(true)).unwrap();
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
        let cut = hierarchy.select_cut(view(true)).unwrap();
        assert!(cut.is_renderable());
        assert_eq!(cut.selected_pages, vec![root.key]);
        assert_eq!(cut.requested_pages.len(), TERRAIN_PAGE_MAX_CHILDREN);
        assert!(cut.ownerless_roots.is_empty());
        assert!(!cut.has_exact_surface_vicinity([-3.2, 3.2, -3.2], 1.0));
    }

    #[test]
    fn complete_group_atomically_replaces_parent_without_overlap() {
        let (mut hierarchy, pages) = hierarchy();
        for page in pages {
            hierarchy.install_page(page).unwrap();
        }
        let cut = hierarchy.select_cut(view(true)).unwrap();
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
        let cut = hierarchy.select_cut(edge_view).unwrap();
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
        let cut = hierarchy.select_cut(view(false)).unwrap();
        assert_eq!(cut.selected_pages.len(), 1);
        assert_eq!(cut.selected_pages[0].level, 1);
        assert_eq!(cut.selected_primitives, 1);
    }

    #[test]
    fn exact_surface_radius_selects_leaf_lattice_without_debug_override() {
        let root = TerrainPageKey::surface(1, -1, -1);
        let (directory, pages) = surface_segment(root);
        let mut hierarchy =
            VirtualTerrainHierarchy::new(VirtualTerrainCapacity::DEVELOPMENT_128_MIB).unwrap();
        hierarchy.register_region_directory(&directory).unwrap();
        for page in pages {
            hierarchy.install_page(page).unwrap();
        }
        let mut exact_view = view(false);
        exact_view.camera_position_metres = [-3.2, 3.2, 1.0];
        exact_view.exact_surface_radius_metres = 4.0;
        let cut = hierarchy.select_cut(exact_view).unwrap();
        assert_eq!(cut.selected_pages.len(), 4);
        assert!(cut.selected_pages.iter().all(|key| key.level == 0));
        assert!(cut.requested_pages.is_empty());
        assert!(cut.has_exact_surface_vicinity(exact_view.camera_position_metres, 4.0));
    }

    #[test]
    fn missing_root_is_explicitly_ownerless_and_requested() {
        let (mut hierarchy, _) = hierarchy();
        let cut = hierarchy.select_cut(view(false)).unwrap();
        assert!(!cut.is_renderable());
        assert_eq!(cut.ownerless_roots.len(), 1);
        assert_eq!(cut.requested_pages.len(), 1);
    }

    #[test]
    fn empty_hierarchy_is_not_a_renderable_owner() {
        let mut hierarchy =
            VirtualTerrainHierarchy::new(VirtualTerrainCapacity::DEVELOPMENT_128_MIB).unwrap();
        let cut = hierarchy.select_cut(view(false)).unwrap();
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
            hierarchy.select_cut(invalid),
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
