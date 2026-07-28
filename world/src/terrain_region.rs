//! Host-side construction of fixed virtual terrain regions.
//!
//! Published parents may be simplified, but every ancestor is rebuilt from a parallel exact
//! surface sidecar. Approximation errors therefore never compound geometrically, and a parent does
//! not need to reverse-engineer canonical occupancy from a child's compact payload.

use crate::terrain_page::{
    assemble_exact_cluster_terrain_parent_from_surfaces, select_budgeted_exact_terrain_parent,
};
use crate::{
    Material, TERRAIN_PAGE_TARGET_COMPRESSED_BYTES, TERRAIN_REGION_ROOT_LEVEL,
    TerrainDirectoryError, TerrainHierarchyDirectoryV1, TerrainPageBuildError,
    TerrainPageCodecError, TerrainPageKey, TerrainPageV1, TerrainSimplificationBudget, VoxelCoord,
    WorldSourceIdentityHash, build_exact_terrain_page, encode_terrain_page,
};
use std::collections::BTreeMap;
use std::fmt;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TerrainRegionBuildV1 {
    pub root: TerrainPageKey,
    pub pages: Vec<TerrainPageV1>,
    pub directory: TerrainHierarchyDirectoryV1,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TerrainRegionBuildError {
    InvalidRoot,
    MissingChild(TerrainPageKey),
    Page(TerrainPageBuildError),
    Codec(TerrainPageCodecError),
    Directory(TerrainDirectoryError),
    PublishedPageOverBudget {
        key: TerrainPageKey,
        encoded_bytes: usize,
    },
}

impl fmt::Display for TerrainRegionBuildError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRoot => formatter.write_str("terrain region root is invalid"),
            Self::MissingChild(key) => write!(formatter, "terrain region is missing child {key:?}"),
            Self::Page(error) => write!(formatter, "terrain region page build failed: {error}"),
            Self::Codec(error) => write!(formatter, "terrain region page codec failed: {error}"),
            Self::Directory(error) => {
                write!(formatter, "terrain region directory build failed: {error}")
            }
            Self::PublishedPageOverBudget { key, encoded_bytes } => write!(
                formatter,
                "terrain region page {key:?} is {encoded_bytes} bytes, above the publication budget"
            ),
        }
    }
}

impl std::error::Error for TerrainRegionBuildError {}

impl From<TerrainPageBuildError> for TerrainRegionBuildError {
    fn from(error: TerrainPageBuildError) -> Self {
        Self::Page(error)
    }
}

impl From<TerrainPageCodecError> for TerrainRegionBuildError {
    fn from(error: TerrainPageCodecError) -> Self {
        Self::Codec(error)
    }
}

impl From<TerrainDirectoryError> for TerrainRegionBuildError {
    fn from(error: TerrainDirectoryError) -> Self {
        Self::Directory(error)
    }
}

/// Builds one complete 12.8 m region hierarchy from canonical occupancy and material.
///
/// Every published page is independently decodable and at most 64 KiB. The returned directory
/// contains the complete tree down to exact 32³ leaves, while the fixed root is immediately usable
/// as the region's last-resident fallback.
pub fn build_terrain_region(
    source_identity_hash: WorldSourceIdentityHash,
    root: TerrainPageKey,
    revision: u64,
    budget: TerrainSimplificationBudget,
    mut material_at: impl FnMut(VoxelCoord) -> Material,
) -> Result<TerrainRegionBuildV1, TerrainRegionBuildError> {
    if root.level != TERRAIN_REGION_ROOT_LEVEL || root.bounds().is_none() {
        return Err(TerrainRegionBuildError::InvalidRoot);
    }

    let mut published = BTreeMap::<TerrainPageKey, TerrainPageV1>::new();
    let mut exact_surfaces = BTreeMap::<TerrainPageKey, TerrainPageV1>::new();
    for key in descendant_keys_at_level(root, 0)? {
        let leaf = build_exact_terrain_page(source_identity_hash, key, revision, &mut material_at)?;
        ensure_publication_budget(&leaf)?;
        exact_surfaces.insert(key, leaf.clone());
        published.insert(key, leaf);
    }

    for level in 1..=TERRAIN_REGION_ROOT_LEVEL {
        for key in descendant_keys_at_level(root, level)? {
            let child_keys = key.children().ok_or(TerrainRegionBuildError::InvalidRoot)?;
            let children = child_keys
                .iter()
                .map(|child| {
                    published
                        .get(child)
                        .cloned()
                        .ok_or(TerrainRegionBuildError::MissingChild(*child))
                })
                .collect::<Result<Vec<_>, _>>()?;
            let exact_children = child_keys
                .iter()
                .map(|child| {
                    exact_surfaces
                        .get(child)
                        .cloned()
                        .ok_or(TerrainRegionBuildError::MissingChild(*child))
                })
                .collect::<Result<Vec<_>, _>>()?;
            let exact = assemble_exact_cluster_terrain_parent_from_surfaces(
                key,
                revision,
                &children,
                &exact_children,
            )?;
            let page = select_budgeted_exact_terrain_parent(&exact, &children, budget)?;
            ensure_publication_budget(&page)?;
            exact_surfaces.insert(key, exact);
            published.insert(key, page);
        }
    }

    let pages = published.into_values().collect::<Vec<_>>();
    let directory = TerrainHierarchyDirectoryV1::from_region_pages(&pages)?;
    Ok(TerrainRegionBuildV1 {
        root,
        pages,
        directory,
    })
}

fn ensure_publication_budget(page: &TerrainPageV1) -> Result<(), TerrainRegionBuildError> {
    let encoded_bytes = encode_terrain_page(page)?.len();
    if encoded_bytes > TERRAIN_PAGE_TARGET_COMPRESSED_BYTES {
        return Err(TerrainRegionBuildError::PublishedPageOverBudget {
            key: page.key,
            encoded_bytes,
        });
    }
    Ok(())
}

fn descendant_keys_at_level(
    root: TerrainPageKey,
    level: u8,
) -> Result<Vec<TerrainPageKey>, TerrainRegionBuildError> {
    if level > root.level {
        return Err(TerrainRegionBuildError::InvalidRoot);
    }
    let mut keys = vec![root];
    while keys.first().is_some_and(|key| key.level > level) {
        let mut children = Vec::with_capacity(keys.len().saturating_mul(8));
        for key in keys {
            children.extend(key.children().ok_or(TerrainRegionBuildError::InvalidRoot)?);
        }
        keys = children;
    }
    keys.sort_unstable();
    Ok(keys)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{TerrainPageRepresentation, decode_terrain_page};

    fn identity() -> WorldSourceIdentityHash {
        WorldSourceIdentityHash::from_bytes([0x37; 32])
    }

    #[test]
    fn region_builder_emits_a_complete_fixed_negative_coordinate_hierarchy() {
        let root = TerrainPageKey {
            level: TERRAIN_REGION_ROOT_LEVEL,
            coord: [-1, -1, 0],
        };
        let surface_y = root.bounds().unwrap().min.y + 127;
        let build = build_terrain_region(
            identity(),
            root,
            19,
            TerrainSimplificationBudget {
                target_triangles: 4_096,
                max_error_millivoxels: 8_000,
                target_encoded_bytes: TERRAIN_PAGE_TARGET_COMPRESSED_BYTES as u32,
            },
            |coord| {
                if coord.y <= surface_y {
                    Material::Stone
                } else {
                    Material::Air
                }
            },
        )
        .unwrap();

        assert_eq!(build.pages.len(), 1 + 8 + 64 + 512);
        assert!(build.directory.validates_region_partition());
        assert_eq!(build.directory.roots().next().unwrap().key, root);
        assert!(
            build
                .pages
                .iter()
                .filter(|page| page.key.level == 0)
                .all(|page| matches!(
                    page.representation,
                    TerrainPageRepresentation::SurfaceCluster(_)
                ))
        );
        for page in &build.pages {
            let encoded = encode_terrain_page(page).unwrap();
            assert!(encoded.len() <= TERRAIN_PAGE_TARGET_COMPRESSED_BYTES);
            assert_eq!(decode_terrain_page(&encoded, identity()).unwrap(), *page);
        }
    }

    #[test]
    fn region_builder_rejects_noncanonical_roots() {
        let error = build_terrain_region(
            identity(),
            TerrainPageKey {
                level: TERRAIN_REGION_ROOT_LEVEL - 1,
                coord: [0, 0, 0],
            },
            1,
            TerrainSimplificationBudget {
                target_triangles: 1,
                max_error_millivoxels: 1,
                target_encoded_bytes: 1,
            },
            |_| Material::Air,
        )
        .unwrap_err();
        assert_eq!(error, TerrainRegionBuildError::InvalidRoot);
    }
}
