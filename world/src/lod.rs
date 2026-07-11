use crate::mesh::{FACE_NEG_X, FACE_NEG_Z, FACE_POS_X, FACE_POS_Y, FACE_POS_Z};
use crate::{Generator, Material};

pub const FAR_STRIDE_VOXELS: i32 = 8;
pub const FAR_TILE_EDGE_CELLS: i32 = 32;
pub const FAR_TILE_SPAN_VOXELS: i32 = FAR_STRIDE_VOXELS * FAR_TILE_EDGE_CELLS;

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
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SurfaceQuad {
    pub origin: [i32; 3],
    pub face: u8,
    pub extent: [u16; 2],
    pub material: Material,
}

/// Builds a closed, surface-preserving coarse shell. Adjacent tile samples use the same global cell
/// centers, and vertical transition quads close every height discontinuity, so independently streamed
/// tiles cannot expose cracks.
pub fn generate_far_tile(generator: Generator, coord: FarTileCoord) -> Vec<SurfaceQuad> {
    generate_far_tile_with(coord, |x, z| {
        let height = generator.surface_height(x, z);
        (height, generator.sample(x, height, z))
    })
}

pub fn generate_far_tile_with(
    coord: FarTileCoord,
    surface: impl Fn(i32, i32) -> (i32, Material),
) -> Vec<SurfaceQuad> {
    let [origin_x, origin_z] = coord.voxel_origin();
    let mut quads = Vec::with_capacity((FAR_TILE_EDGE_CELLS * FAR_TILE_EDGE_CELLS * 3) as usize);
    for cell_z in 0..FAR_TILE_EDGE_CELLS {
        for cell_x in 0..FAR_TILE_EDGE_CELLS {
            let x = origin_x + cell_x * FAR_STRIDE_VOXELS;
            let z = origin_z + cell_z * FAR_STRIDE_VOXELS;
            let center_x = x + FAR_STRIDE_VOXELS / 2;
            let center_z = z + FAR_STRIDE_VOXELS / 2;
            let (height, material) = surface(center_x, center_z);
            quads.push(SurfaceQuad {
                origin: [x, height, z],
                face: FACE_POS_Y,
                extent: [FAR_STRIDE_VOXELS as u16; 2],
                material,
            });

            let neighbors = [
                (-1, 0, FACE_NEG_X),
                (1, 0, FACE_POS_X),
                (0, -1, FACE_NEG_Z),
                (0, 1, FACE_POS_Z),
            ];
            for (dx, dz, face) in neighbors {
                let (neighbor_height, _) = surface(
                    center_x + dx * FAR_STRIDE_VOXELS,
                    center_z + dz * FAR_STRIDE_VOXELS,
                );
                if height <= neighbor_height {
                    continue;
                }
                let side_origin = match face {
                    FACE_POS_X => [x + FAR_STRIDE_VOXELS - 1, neighbor_height + 1, z],
                    FACE_NEG_X => [x, neighbor_height + 1, z],
                    FACE_POS_Z => [x, neighbor_height + 1, z + FAR_STRIDE_VOXELS - 1],
                    _ => [x, neighbor_height + 1, z],
                };
                quads.push(SurfaceQuad {
                    origin: side_origin,
                    face,
                    extent: [FAR_STRIDE_VOXELS as u16, (height - neighbor_height) as u16],
                    material,
                });
            }
        }
    }
    quads
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn far_tiles_cover_exactly_their_canonical_span() {
        assert_eq!(FarTileCoord::new(-1, 2).voxel_origin(), [-256, 512]);
        let tile = generate_far_tile(Generator::new(7), FarTileCoord::new(0, 0));
        assert_eq!(
            tile.iter().filter(|quad| quad.face == FACE_POS_Y).count(),
            (FAR_TILE_EDGE_CELLS * FAR_TILE_EDGE_CELLS) as usize
        );
    }

    #[test]
    fn independently_generated_neighbors_share_boundary_heights() {
        let generator = Generator::new(0x5eed);
        let left = generate_far_tile(generator, FarTileCoord::new(0, 0));
        let right = generate_far_tile(generator, FarTileCoord::new(1, 0));
        let left_edge = left.iter().filter(|quad| {
            quad.face == FACE_POS_Y && quad.origin[0] == FAR_TILE_SPAN_VOXELS - FAR_STRIDE_VOXELS
        });
        let right_edge: Vec<_> = right
            .iter()
            .filter(|quad| quad.face == FACE_POS_Y && quad.origin[0] == FAR_TILE_SPAN_VOXELS)
            .collect();
        for (index, quad) in left_edge.enumerate() {
            let adjacent_height = generator.surface_height(
                FAR_TILE_SPAN_VOXELS + FAR_STRIDE_VOXELS / 2,
                index as i32 * FAR_STRIDE_VOXELS + FAR_STRIDE_VOXELS / 2,
            );
            assert_eq!(right_edge[index].origin[1], adjacent_height);
            assert_eq!(
                quad.origin[0] + FAR_STRIDE_VOXELS,
                right_edge[index].origin[0]
            );
        }
    }
}
