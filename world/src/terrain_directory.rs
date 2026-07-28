//! Compact, versioned directory for a finite virtual-terrain hierarchy region.
//!
//! The directory is metadata, not residency. It describes a complete page forest so traversal can
//! select a valid ancestor while individual payloads move through memory and persistent caches.

use crate::{
    TERRAIN_PAGE_MAX_COMPRESSED_BYTES, TERRAIN_PAGE_MAX_LEVEL,
    TERRAIN_PAGE_TARGET_COMPRESSED_BYTES, TerrainErrorBounds, TerrainPageCodecError,
    TerrainPageKey, TerrainPageRepresentationKind, TerrainPageV1, TerrainTopologyClass,
    WorldSourceIdentityHash, encode_terrain_page,
};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

pub const TERRAIN_DIRECTORY_SCHEMA_VERSION: u16 = 1;
pub const TERRAIN_DIRECTORY_MAX_NODES: usize = 131_072;
pub const TERRAIN_DIRECTORY_MAX_ROOTS: usize = 4_096;
/// Production region roots cover a fixed 12.8 m cube (128 canonical 10 cm voxels per edge).
///
/// A complete region contains 64 exact 32³ leaves plus their 8 parents and one root. This keeps the
/// first usable parent cheap enough to build inside the browser's request deadline while retaining
/// independently cacheable fixed roots. The world is a forest of these roots; it is deliberately
/// not one octree that grows until it encloses arbitrary coordinates.
pub const TERRAIN_REGION_ROOT_LEVEL: u8 = 2;
const DIRECTORY_MAGIC: &[u8; 4] = b"VXTD";
const DIRECTORY_HEADER_BYTES: usize = 80;
const DIRECTORY_NODE_BYTES: usize = 80;
const DIRECTORY_FINGERPRINT_DOMAIN: &[u8] = b"voxels-terrain-directory-v1\0";
const NODE_FLAG_HAS_CHILDREN: u8 = 1 << 0;
const NODE_FLAG_ROOT: u8 = 1 << 1;
const NODE_FLAG_UNRESOLVED_TOPOLOGY: u8 = 1 << 2;
const NODE_FLAGS_KNOWN: u8 =
    NODE_FLAG_HAS_CHILDREN | NODE_FLAG_ROOT | NODE_FLAG_UNRESOLVED_TOPOLOGY;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TerrainHierarchyNode {
    pub key: TerrainPageKey,
    pub revision: u64,
    pub content_fingerprint: [u8; 32],
    pub errors: TerrainErrorBounds,
    pub topology: TerrainTopologyClass,
    pub representation: TerrainPageRepresentationKind,
    pub encoded_bytes: u32,
    pub has_children: bool,
    pub is_root: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TerrainHierarchyDirectoryV1 {
    pub source_identity_hash: WorldSourceIdentityHash,
    pub nodes: Vec<TerrainHierarchyNode>,
    pub content_fingerprint: [u8; 32],
}

impl TerrainHierarchyDirectoryV1 {
    pub fn from_pages(pages: &[TerrainPageV1]) -> Result<Self, TerrainDirectoryError> {
        if pages.is_empty() {
            return Err(TerrainDirectoryError::Empty);
        }
        if pages.len() > TERRAIN_DIRECTORY_MAX_NODES {
            return Err(TerrainDirectoryError::LimitExceeded("node count"));
        }
        if pages.iter().any(|page| !page.validates_identity()) {
            return Err(TerrainDirectoryError::InvalidPage);
        }
        let source_identity_hash = pages[0].source_identity_hash;
        if pages
            .iter()
            .any(|page| page.source_identity_hash != source_identity_hash)
        {
            return Err(TerrainDirectoryError::SourceMismatch);
        }
        let page_by_key = pages
            .iter()
            .map(|page| (page.key, page))
            .collect::<BTreeMap<_, _>>();
        if page_by_key.len() != pages.len() {
            return Err(TerrainDirectoryError::DuplicateKey);
        }
        let mut referenced = BTreeSet::new();
        for page in pages {
            for child in &page.children {
                let Some(actual) = page_by_key.get(&child.key) else {
                    return Err(TerrainDirectoryError::MissingChild);
                };
                if actual.revision != child.revision
                    || actual.content_fingerprint != child.content_fingerprint
                {
                    return Err(TerrainDirectoryError::ChildIdentityMismatch);
                }
                referenced.insert(child.key);
            }
        }
        let mut nodes = Vec::with_capacity(pages.len());
        for page in page_by_key.values() {
            let encoded_bytes = u32::try_from(encode_terrain_page(page)?.len())
                .map_err(|_| TerrainDirectoryError::LimitExceeded("encoded page bytes"))?;
            nodes.push(TerrainHierarchyNode {
                key: page.key,
                revision: page.revision,
                content_fingerprint: page.content_fingerprint,
                errors: page.errors,
                topology: page.topology,
                representation: page.representation.kind(),
                encoded_bytes,
                has_children: !page.children.is_empty(),
                is_root: !referenced.contains(&page.key),
            });
        }
        let mut directory = Self {
            source_identity_hash,
            nodes,
            content_fingerprint: [0; 32],
        };
        directory.content_fingerprint = directory_fingerprint(&directory);
        if !directory.validates_identity() {
            return Err(TerrainDirectoryError::InvalidHierarchy);
        }
        Ok(directory)
    }

    /// Builds the production directory form: a forest of fixed, independently cacheable region
    /// roots with complete refinement to exact level-0 leaves.
    pub fn from_region_pages(pages: &[TerrainPageV1]) -> Result<Self, TerrainDirectoryError> {
        let directory = Self::from_pages(pages)?;
        if !directory.validates_region_partition() {
            return Err(TerrainDirectoryError::InvalidRegionPartition);
        }
        Ok(directory)
    }

    pub fn validates_identity(&self) -> bool {
        if self.nodes.is_empty()
            || self.nodes.len() > TERRAIN_DIRECTORY_MAX_NODES
            || !self.nodes.windows(2).all(|pair| pair[0].key < pair[1].key)
        {
            return false;
        }
        let by_key = self
            .nodes
            .iter()
            .map(|node| (node.key, node))
            .collect::<BTreeMap<_, _>>();
        if by_key.len() != self.nodes.len() {
            return false;
        }
        let roots = self.nodes.iter().filter(|node| node.is_root).count();
        if roots == 0 || roots > TERRAIN_DIRECTORY_MAX_ROOTS {
            return false;
        }
        let mut referenced = BTreeSet::new();
        for node in &self.nodes {
            if node.key.level > TERRAIN_PAGE_MAX_LEVEL
                || node.key.bounds().is_none()
                || node.encoded_bytes == 0
                || usize::try_from(node.encoded_bytes).unwrap_or(usize::MAX)
                    > TERRAIN_PAGE_MAX_COMPRESSED_BYTES + 4_096
            {
                return false;
            }
            let Some(children) = node.key.children().filter(|_| node.has_children) else {
                if node.has_children {
                    return false;
                }
                continue;
            };
            for key in children {
                let Some(child) = by_key.get(&key) else {
                    return false;
                };
                if !errors_cover(node.errors, child.errors) || !referenced.insert(key) {
                    return false;
                }
            }
        }
        if self
            .nodes
            .iter()
            .any(|node| node.is_root == referenced.contains(&node.key))
        {
            return false;
        }
        self.content_fingerprint == directory_fingerprint(self)
    }

    pub fn roots(&self) -> impl Iterator<Item = &TerrainHierarchyNode> {
        self.nodes.iter().filter(|node| node.is_root)
    }

    pub fn validates_region_partition(&self) -> bool {
        if !self.validates_identity()
            || self
                .nodes
                .iter()
                .any(|node| node.key.level > TERRAIN_REGION_ROOT_LEVEL)
        {
            return false;
        }
        let roots = self
            .roots()
            .map(|root| (root.key, root))
            .collect::<BTreeMap<_, _>>();
        if roots
            .values()
            .any(|root| root.key.level != TERRAIN_REGION_ROOT_LEVEL)
        {
            return false;
        }
        self.nodes.iter().all(|node| {
            usize::try_from(node.encoded_bytes)
                .is_ok_and(|bytes| bytes <= TERRAIN_PAGE_TARGET_COMPRESSED_BYTES)
                && node
                    .key
                    .ancestor_at(TERRAIN_REGION_ROOT_LEVEL)
                    .is_some_and(|root| roots.contains_key(&root))
                && (node.has_children || node.key.level == 0)
        })
    }

    pub fn node(&self, key: TerrainPageKey) -> Option<&TerrainHierarchyNode> {
        self.nodes
            .binary_search_by_key(&key, |node| node.key)
            .ok()
            .and_then(|index| self.nodes.get(index))
    }
}

fn errors_cover(parent: TerrainErrorBounds, child: TerrainErrorBounds) -> bool {
    parent.geometric_millivoxels >= child.geometric_millivoxels
        && parent.silhouette_millivoxels >= child.silhouette_millivoxels
        && parent.material_boundary_millivoxels >= child.material_boundary_millivoxels
        && parent.normal_milliradians >= child.normal_milliradians
        && (parent.unresolved_topology || !child.unresolved_topology)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TerrainDirectoryError {
    Empty,
    InvalidPage,
    InvalidHierarchy,
    InvalidRegionPartition,
    SourceMismatch,
    DuplicateKey,
    MissingChild,
    ChildIdentityMismatch,
    Truncated,
    InvalidMagic,
    UnsupportedVersion(u16),
    InvalidHeader(&'static str),
    UnknownTopology(u8),
    UnknownRepresentation(u8),
    LimitExceeded(&'static str),
    CorruptHash,
    PageCodec(TerrainPageCodecError),
}

impl fmt::Display for TerrainDirectoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("terrain directory is empty"),
            Self::InvalidPage => formatter.write_str("terrain directory contains an invalid page"),
            Self::InvalidHierarchy => formatter.write_str("terrain directory hierarchy is invalid"),
            Self::InvalidRegionPartition => {
                formatter.write_str("terrain directory is not a complete fixed region forest")
            }
            Self::SourceMismatch => formatter.write_str("terrain directory page sources differ"),
            Self::DuplicateKey => formatter.write_str("terrain directory contains duplicate keys"),
            Self::MissingChild => formatter.write_str("terrain directory omits a referenced child"),
            Self::ChildIdentityMismatch => formatter
                .write_str("terrain directory child identity differs from parent reference"),
            Self::Truncated => formatter.write_str("truncated VXTD payload"),
            Self::InvalidMagic => formatter.write_str("invalid VXTD magic"),
            Self::UnsupportedVersion(version) => {
                write!(formatter, "unsupported VXTD version {version}")
            }
            Self::InvalidHeader(reason) => write!(formatter, "invalid VXTD header: {reason}"),
            Self::UnknownTopology(value) => write!(formatter, "unknown VXTD topology {value}"),
            Self::UnknownRepresentation(value) => {
                write!(formatter, "unknown VXTD representation {value}")
            }
            Self::LimitExceeded(limit) => write!(formatter, "VXTD limit exceeded: {limit}"),
            Self::CorruptHash => formatter.write_str("VXTD semantic content hash mismatch"),
            Self::PageCodec(error) => write!(formatter, "terrain page encoding failed: {error}"),
        }
    }
}

impl std::error::Error for TerrainDirectoryError {}

impl From<TerrainPageCodecError> for TerrainDirectoryError {
    fn from(error: TerrainPageCodecError) -> Self {
        Self::PageCodec(error)
    }
}

pub fn encode_terrain_directory(
    directory: &TerrainHierarchyDirectoryV1,
) -> Result<Vec<u8>, TerrainDirectoryError> {
    if !directory.validates_identity() {
        return Err(TerrainDirectoryError::InvalidHierarchy);
    }
    let root_count = directory.roots().count();
    let mut encoded =
        Vec::with_capacity(DIRECTORY_HEADER_BYTES + directory.nodes.len() * DIRECTORY_NODE_BYTES);
    encoded.extend_from_slice(DIRECTORY_MAGIC);
    push_u16(&mut encoded, TERRAIN_DIRECTORY_SCHEMA_VERSION);
    push_u16(&mut encoded, DIRECTORY_HEADER_BYTES as u16);
    encoded.extend_from_slice(directory.source_identity_hash.as_bytes());
    push_u32(&mut encoded, directory.nodes.len() as u32);
    push_u32(&mut encoded, root_count as u32);
    encoded.extend_from_slice(&directory.content_fingerprint);
    debug_assert_eq!(encoded.len(), DIRECTORY_HEADER_BYTES);
    for node in &directory.nodes {
        encoded.push(node.key.level);
        let mut flags = 0u8;
        flags |= u8::from(node.has_children) * NODE_FLAG_HAS_CHILDREN;
        flags |= u8::from(node.is_root) * NODE_FLAG_ROOT;
        flags |= u8::from(node.errors.unresolved_topology) * NODE_FLAG_UNRESOLVED_TOPOLOGY;
        encoded.push(flags);
        encoded.push(node.topology as u8);
        encoded.push(node.representation as u8);
        for component in node.key.coord {
            push_i32(&mut encoded, component);
        }
        push_u64(&mut encoded, node.revision);
        encoded.extend_from_slice(&node.content_fingerprint);
        push_u32(&mut encoded, node.errors.geometric_millivoxels);
        push_u32(&mut encoded, node.errors.silhouette_millivoxels);
        push_u32(&mut encoded, node.errors.material_boundary_millivoxels);
        push_u32(&mut encoded, node.errors.normal_milliradians);
        push_u32(&mut encoded, node.encoded_bytes);
        push_u32(&mut encoded, 0);
    }
    Ok(encoded)
}

pub fn decode_terrain_directory(
    encoded: &[u8],
    expected_source: WorldSourceIdentityHash,
) -> Result<TerrainHierarchyDirectoryV1, TerrainDirectoryError> {
    if encoded.len() < DIRECTORY_HEADER_BYTES {
        return Err(TerrainDirectoryError::Truncated);
    }
    let mut cursor = Cursor::new(encoded);
    if cursor.take(4)? != DIRECTORY_MAGIC {
        return Err(TerrainDirectoryError::InvalidMagic);
    }
    let version = cursor.u16()?;
    if version != TERRAIN_DIRECTORY_SCHEMA_VERSION {
        return Err(TerrainDirectoryError::UnsupportedVersion(version));
    }
    if usize::from(cursor.u16()?) != DIRECTORY_HEADER_BYTES {
        return Err(TerrainDirectoryError::InvalidHeader("header size"));
    }
    let source_identity_hash = WorldSourceIdentityHash::from_bytes(cursor.array::<32>()?);
    if source_identity_hash != expected_source {
        return Err(TerrainDirectoryError::SourceMismatch);
    }
    let node_count = usize::try_from(cursor.u32()?)
        .map_err(|_| TerrainDirectoryError::LimitExceeded("node count"))?;
    let root_count = usize::try_from(cursor.u32()?)
        .map_err(|_| TerrainDirectoryError::LimitExceeded("root count"))?;
    if node_count == 0 || node_count > TERRAIN_DIRECTORY_MAX_NODES {
        return Err(TerrainDirectoryError::LimitExceeded("node count"));
    }
    if root_count == 0 || root_count > TERRAIN_DIRECTORY_MAX_ROOTS {
        return Err(TerrainDirectoryError::LimitExceeded("root count"));
    }
    let content_fingerprint = cursor.array::<32>()?;
    let expected_len = DIRECTORY_HEADER_BYTES
        .checked_add(
            node_count
                .checked_mul(DIRECTORY_NODE_BYTES)
                .ok_or(TerrainDirectoryError::LimitExceeded("encoded bytes"))?,
        )
        .ok_or(TerrainDirectoryError::LimitExceeded("encoded bytes"))?;
    if encoded.len() != expected_len {
        return Err(if encoded.len() < expected_len {
            TerrainDirectoryError::Truncated
        } else {
            TerrainDirectoryError::InvalidHeader("trailing bytes")
        });
    }
    let mut nodes = Vec::with_capacity(node_count);
    for _ in 0..node_count {
        let level = cursor.u8()?;
        let flags = cursor.u8()?;
        if flags & !NODE_FLAGS_KNOWN != 0 {
            return Err(TerrainDirectoryError::InvalidHeader("node flags"));
        }
        let topology = match cursor.u8()? {
            0 => TerrainTopologyClass::SingleRunColumns,
            1 => TerrainTopologyClass::Volumetric,
            value => return Err(TerrainDirectoryError::UnknownTopology(value)),
        };
        let representation = match cursor.u8()? {
            1 => TerrainPageRepresentationKind::SteppedSurfaceResidual,
            2 => TerrainPageRepresentationKind::SparseVoxelBrick,
            3 => TerrainPageRepresentationKind::SurfaceCluster,
            4 => TerrainPageRepresentationKind::TriangleCluster,
            value => return Err(TerrainDirectoryError::UnknownRepresentation(value)),
        };
        let key = TerrainPageKey {
            level,
            coord: [cursor.i32()?, cursor.i32()?, cursor.i32()?],
        };
        let revision = cursor.u64()?;
        let page_fingerprint = cursor.array::<32>()?;
        let errors = TerrainErrorBounds {
            geometric_millivoxels: cursor.u32()?,
            silhouette_millivoxels: cursor.u32()?,
            material_boundary_millivoxels: cursor.u32()?,
            normal_milliradians: cursor.u32()?,
            unresolved_topology: flags & NODE_FLAG_UNRESOLVED_TOPOLOGY != 0,
        };
        let encoded_bytes = cursor.u32()?;
        if cursor.u32()? != 0 {
            return Err(TerrainDirectoryError::InvalidHeader("node reserved bytes"));
        }
        nodes.push(TerrainHierarchyNode {
            key,
            revision,
            content_fingerprint: page_fingerprint,
            errors,
            topology,
            representation,
            encoded_bytes,
            has_children: flags & NODE_FLAG_HAS_CHILDREN != 0,
            is_root: flags & NODE_FLAG_ROOT != 0,
        });
    }
    let directory = TerrainHierarchyDirectoryV1 {
        source_identity_hash,
        nodes,
        content_fingerprint,
    };
    if directory.roots().count() != root_count {
        return Err(TerrainDirectoryError::InvalidHeader("root count"));
    }
    if directory.content_fingerprint != directory_fingerprint(&directory) {
        return Err(TerrainDirectoryError::CorruptHash);
    }
    if !directory.validates_identity() {
        return Err(TerrainDirectoryError::InvalidHierarchy);
    }
    Ok(directory)
}

pub fn decode_region_terrain_directory(
    encoded: &[u8],
    expected_source: WorldSourceIdentityHash,
) -> Result<TerrainHierarchyDirectoryV1, TerrainDirectoryError> {
    let directory = decode_terrain_directory(encoded, expected_source)?;
    if !directory.validates_region_partition() {
        return Err(TerrainDirectoryError::InvalidRegionPartition);
    }
    Ok(directory)
}

fn directory_fingerprint(directory: &TerrainHierarchyDirectoryV1) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(DIRECTORY_FINGERPRINT_DOMAIN);
    hasher.update(&TERRAIN_DIRECTORY_SCHEMA_VERSION.to_le_bytes());
    hasher.update(directory.source_identity_hash.as_bytes());
    hasher.update(&(directory.nodes.len() as u32).to_le_bytes());
    for node in &directory.nodes {
        hasher.update(&[node.key.level]);
        for component in node.key.coord {
            hasher.update(&component.to_le_bytes());
        }
        hasher.update(&node.revision.to_le_bytes());
        hasher.update(&node.content_fingerprint);
        hasher.update(&node.errors.geometric_millivoxels.to_le_bytes());
        hasher.update(&node.errors.silhouette_millivoxels.to_le_bytes());
        hasher.update(&node.errors.material_boundary_millivoxels.to_le_bytes());
        hasher.update(&node.errors.normal_milliradians.to_le_bytes());
        hasher.update(&[u8::from(node.errors.unresolved_topology)]);
        hasher.update(&[
            node.topology as u8,
            node.representation as u8,
            u8::from(node.has_children),
            u8::from(node.is_root),
        ]);
        hasher.update(&node.encoded_bytes.to_le_bytes());
    }
    *hasher.finalize().as_bytes()
}

fn push_u16(bytes: &mut Vec<u8>, value: u16) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn push_u32(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn push_i32(bytes: &mut Vec<u8>, value: i32) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn push_u64(bytes: &mut Vec<u8>, value: u64) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

struct Cursor<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> Cursor<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    fn take(&mut self, count: usize) -> Result<&'a [u8], TerrainDirectoryError> {
        let end = self
            .offset
            .checked_add(count)
            .ok_or(TerrainDirectoryError::Truncated)?;
        let slice = self
            .bytes
            .get(self.offset..end)
            .ok_or(TerrainDirectoryError::Truncated)?;
        self.offset = end;
        Ok(slice)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], TerrainDirectoryError> {
        self.take(N)?
            .try_into()
            .map_err(|_| TerrainDirectoryError::Truncated)
    }

    fn u8(&mut self) -> Result<u8, TerrainDirectoryError> {
        Ok(self.array::<1>()?[0])
    }

    fn u16(&mut self) -> Result<u16, TerrainDirectoryError> {
        Ok(u16::from_le_bytes(self.array()?))
    }

    fn u32(&mut self) -> Result<u32, TerrainDirectoryError> {
        Ok(u32::from_le_bytes(self.array()?))
    }

    fn i32(&mut self) -> Result<i32, TerrainDirectoryError> {
        Ok(i32::from_le_bytes(self.array()?))
    }

    fn u64(&mut self) -> Result<u64, TerrainDirectoryError> {
        Ok(u64::from_le_bytes(self.array()?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Material, TerrainPageRepresentation, VoxelCoord, build_exact_cluster_terrain_parent,
        build_exact_terrain_page,
    };

    fn identity() -> WorldSourceIdentityHash {
        WorldSourceIdentityHash::from_bytes([0x8c; 32])
    }

    fn terrain(coord: VoxelCoord) -> Material {
        let height = (coord.x.div_euclid(5) + coord.z.div_euclid(7)).rem_euclid(12) - 36;
        if coord.y > height {
            Material::Air
        } else if coord.y == height {
            Material::Grass
        } else {
            Material::Stone
        }
    }

    fn page_forest() -> Vec<TerrainPageV1> {
        let root_key = TerrainPageKey {
            level: 1,
            coord: [-1, -1, 0],
        };
        let leaves = root_key
            .children()
            .unwrap()
            .into_iter()
            .map(|key| build_exact_terrain_page(identity(), key, 7, terrain).unwrap())
            .collect::<Vec<_>>();
        let root = build_exact_cluster_terrain_parent(root_key, 8, &leaves).unwrap();
        leaves.into_iter().chain([root]).collect()
    }

    fn structural_region_forest(root_coords: &[[i32; 3]]) -> TerrainHierarchyDirectoryV1 {
        let root_keys = root_coords
            .iter()
            .copied()
            .map(|coord| TerrainPageKey {
                level: TERRAIN_REGION_ROOT_LEVEL,
                coord,
            })
            .collect::<BTreeSet<_>>();
        let mut pending = root_keys.iter().copied().collect::<Vec<_>>();
        let mut keys = BTreeSet::new();
        while let Some(key) = pending.pop() {
            if !keys.insert(key) {
                continue;
            }
            if let Some(children) = key.children() {
                pending.extend(children);
            }
        }
        let nodes = keys
            .into_iter()
            .map(|key| TerrainHierarchyNode {
                key,
                revision: 9,
                content_fingerprint: *blake3::hash(format!("{key:?}").as_bytes()).as_bytes(),
                errors: TerrainErrorBounds::EXACT,
                topology: TerrainTopologyClass::Volumetric,
                representation: TerrainPageRepresentationKind::SurfaceCluster,
                encoded_bytes: 1_024,
                has_children: key.level > 0,
                is_root: root_keys.contains(&key),
            })
            .collect();
        let mut directory = TerrainHierarchyDirectoryV1 {
            source_identity_hash: identity(),
            nodes,
            content_fingerprint: [0; 32],
        };
        directory.content_fingerprint = directory_fingerprint(&directory);
        assert!(directory.validates_identity());
        directory
    }

    #[test]
    fn directory_round_trips_a_complete_negative_coordinate_forest() {
        let pages = page_forest();
        let directory = TerrainHierarchyDirectoryV1::from_pages(&pages).unwrap();
        assert!(directory.validates_identity());
        assert_eq!(directory.roots().count(), 1);
        assert_eq!(directory.nodes.len(), 9);
        let root = directory.roots().next().unwrap();
        assert!(root.has_children);
        assert_eq!(root.key.coord, [-1, -1, 0]);
        assert!(matches!(
            root.representation,
            TerrainPageRepresentationKind::SurfaceCluster
        ));
        let encoded = encode_terrain_directory(&directory).unwrap();
        assert_eq!(
            decode_terrain_directory(&encoded, identity()).unwrap(),
            directory
        );
        assert_eq!(encode_terrain_directory(&directory).unwrap(), encoded);
    }

    #[test]
    fn directory_rejects_missing_children_and_optimistic_parent_errors() {
        let pages = page_forest();
        assert_eq!(
            TerrainHierarchyDirectoryV1::from_pages(&pages[1..]),
            Err(TerrainDirectoryError::MissingChild)
        );

        let mut directory = TerrainHierarchyDirectoryV1::from_pages(&pages).unwrap();
        let leaf_index = directory
            .nodes
            .iter()
            .position(|node| !node.has_children)
            .unwrap();
        directory.nodes[leaf_index].errors.geometric_millivoxels = 1;
        directory.content_fingerprint = directory_fingerprint(&directory);
        assert!(!directory.validates_identity());
    }

    #[test]
    fn directory_codec_rejects_wrong_source_corruption_and_trailing_bytes() {
        let directory = TerrainHierarchyDirectoryV1::from_pages(&page_forest()).unwrap();
        let encoded = encode_terrain_directory(&directory).unwrap();
        assert_eq!(
            decode_terrain_directory(&encoded, WorldSourceIdentityHash::from_bytes([0x1d; 32])),
            Err(TerrainDirectoryError::SourceMismatch)
        );
        let mut corrupted = encoded.clone();
        let last = corrupted.len() - 1;
        corrupted[last] ^= 0x40;
        assert_eq!(
            decode_terrain_directory(&corrupted, identity()),
            Err(TerrainDirectoryError::InvalidHeader("node reserved bytes"))
        );
        let mut trailing = encoded;
        trailing.push(0);
        assert_eq!(
            decode_terrain_directory(&trailing, identity()),
            Err(TerrainDirectoryError::InvalidHeader("trailing bytes"))
        );
    }

    #[test]
    fn directory_records_actual_payload_kinds_and_sizes() {
        let pages = page_forest();
        let directory = TerrainHierarchyDirectoryV1::from_pages(&pages).unwrap();
        for node in &directory.nodes {
            let page = pages.iter().find(|page| page.key == node.key).unwrap();
            assert_eq!(node.representation, page.representation.kind());
            assert_eq!(
                usize::try_from(node.encoded_bytes).unwrap(),
                encode_terrain_page(page).unwrap().len()
            );
            assert!(matches!(
                page.representation,
                TerrainPageRepresentation::SurfaceCluster(_)
            ));
        }
    }

    #[test]
    fn fixed_region_forest_is_complete_bounded_and_negative_coordinate_safe() {
        let directory = structural_region_forest(&[[-1, -1, -1], [0, -1, -1]]);
        assert!(directory.validates_region_partition());
        assert_eq!(directory.roots().count(), 2);
        let nodes_per_root = (0..=u32::from(TERRAIN_REGION_ROOT_LEVEL))
            .map(|level| 8_usize.pow(level))
            .sum::<usize>();
        assert_eq!(directory.nodes.len(), 2 * nodes_per_root);
        let encoded = encode_terrain_directory(&directory).unwrap();
        assert_eq!(
            decode_region_terrain_directory(&encoded, identity()).unwrap(),
            directory
        );

        let mut oversized = directory.clone();
        oversized.nodes[0].encoded_bytes =
            u32::try_from(TERRAIN_PAGE_TARGET_COMPRESSED_BYTES + 1).unwrap();
        oversized.content_fingerprint = directory_fingerprint(&oversized);
        assert!(oversized.validates_identity());
        assert!(!oversized.validates_region_partition());
    }

    #[test]
    fn production_region_directory_rejects_arbitrary_or_terminal_coarse_roots() {
        assert_eq!(
            TerrainHierarchyDirectoryV1::from_region_pages(&page_forest()),
            Err(TerrainDirectoryError::InvalidRegionPartition)
        );

        let key = TerrainPageKey {
            level: TERRAIN_REGION_ROOT_LEVEL,
            coord: [-2, 0, 3],
        };
        let mut terminal = TerrainHierarchyDirectoryV1 {
            source_identity_hash: identity(),
            nodes: vec![TerrainHierarchyNode {
                key,
                revision: 1,
                content_fingerprint: [7; 32],
                errors: TerrainErrorBounds {
                    unresolved_topology: true,
                    ..TerrainErrorBounds::EXACT
                },
                topology: TerrainTopologyClass::Volumetric,
                representation: TerrainPageRepresentationKind::SurfaceCluster,
                encoded_bytes: 1_024,
                has_children: false,
                is_root: true,
            }],
            content_fingerprint: [0; 32],
        };
        terminal.content_fingerprint = directory_fingerprint(&terminal);
        assert!(terminal.validates_identity());
        assert!(!terminal.validates_region_partition());
    }
}
