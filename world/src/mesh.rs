use crate::{CHUNK_EDGE, Chunk, Material};
use bytemuck::{Pod, Zeroable};

pub const FACE_POS_X: u8 = 0;
pub const FACE_NEG_X: u8 = 1;
pub const FACE_POS_Y: u8 = 2;
pub const FACE_NEG_Y: u8 = 3;
pub const FACE_POS_Z: u8 = 4;
pub const FACE_NEG_Z: u8 = 5;

/// Compact durable mesher output. The renderer expands each record into six vertices on the GPU.
#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, Pod, Zeroable)]
pub struct Quad {
    pub origin: [u8; 3],
    pub face: u8,
    pub extent: [u8; 2],
    pub material: u16,
}

const _: () = assert!(size_of::<Quad>() == 8);

/// Greedily merges coplanar visible faces with the same material. `outside` samples world coordinates
/// beyond this chunk, preventing hidden seam faces when neighboring chunks are resident or generated.
pub fn mesh_chunk(chunk: &Chunk, outside: impl Fn(i32, i32, i32) -> Material) -> Vec<Quad> {
    let mut quads = Vec::new();
    let origin = chunk.coord().world_origin();
    let mut mask = vec![Material::Air; CHUNK_EDGE * CHUNK_EDGE];
    for face in 0..6 {
        let (axis, u_axis, v_axis, step) = face_axes(face);
        for slice in 0..CHUNK_EDGE {
            for v in 0..CHUNK_EDGE {
                for u in 0..CHUNK_EDGE {
                    let local = compose(axis, u_axis, v_axis, slice, u, v);
                    let material = chunk.get(local[0], local[1], local[2]);
                    if !material.is_solid() {
                        mask[u + v * CHUNK_EDGE] = Material::Air;
                        continue;
                    }
                    let mut neighbor = [local[0] as i32, local[1] as i32, local[2] as i32];
                    neighbor[axis] += step;
                    let adjacent = if neighbor[axis] >= 0 && neighbor[axis] < CHUNK_EDGE as i32 {
                        chunk.get(
                            neighbor[0] as usize,
                            neighbor[1] as usize,
                            neighbor[2] as usize,
                        )
                    } else {
                        outside(
                            origin[0] + neighbor[0],
                            origin[1] + neighbor[1],
                            origin[2] + neighbor[2],
                        )
                    };
                    mask[u + v * CHUNK_EDGE] = if adjacent.is_solid() {
                        Material::Air
                    } else {
                        material
                    };
                }
            }

            let mut v = 0;
            while v < CHUNK_EDGE {
                let mut u = 0;
                while u < CHUNK_EDGE {
                    let material = mask[u + v * CHUNK_EDGE];
                    if !material.is_solid() {
                        u += 1;
                        continue;
                    }
                    let mut width = 1;
                    while u + width < CHUNK_EDGE && mask[u + width + v * CHUNK_EDGE] == material {
                        width += 1;
                    }
                    let mut height = 1;
                    'height: while v + height < CHUNK_EDGE {
                        for offset in 0..width {
                            if mask[u + offset + (v + height) * CHUNK_EDGE] != material {
                                break 'height;
                            }
                        }
                        height += 1;
                    }
                    for clear_v in v..v + height {
                        for clear_u in u..u + width {
                            mask[clear_u + clear_v * CHUNK_EDGE] = Material::Air;
                        }
                    }
                    let local = compose(axis, u_axis, v_axis, slice, u, v);
                    quads.push(Quad {
                        origin: [local[0] as u8, local[1] as u8, local[2] as u8],
                        face,
                        extent: [width as u8, height as u8],
                        material: material.id(),
                    });
                    u += width;
                }
                v += 1;
            }
        }
    }
    quads
}

fn face_axes(face: u8) -> (usize, usize, usize, i32) {
    match face {
        FACE_POS_X => (0, 2, 1, 1),
        FACE_NEG_X => (0, 2, 1, -1),
        FACE_POS_Y => (1, 0, 2, 1),
        FACE_NEG_Y => (1, 0, 2, -1),
        FACE_POS_Z => (2, 0, 1, 1),
        FACE_NEG_Z => (2, 0, 1, -1),
        _ => unreachable!("face is generated from the fixed 0..6 range"),
    }
}

fn compose(
    axis: usize,
    u_axis: usize,
    v_axis: usize,
    slice: usize,
    u: usize,
    v: usize,
) -> [usize; 3] {
    let mut local = [0; 3];
    local[axis] = slice;
    local[u_axis] = u;
    local[v_axis] = v;
    local
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ChunkCoord;

    #[test]
    fn filled_chunk_merges_to_six_quads() {
        let chunk = Chunk::filled(ChunkCoord::new(0, 0, 0), Material::Stone);
        let quads = mesh_chunk(&chunk, |_, _, _| Material::Air);
        assert_eq!(quads.len(), 6);
        assert!(quads.iter().all(|quad| quad.extent == [32, 32]));
    }

    #[test]
    fn solid_neighbor_suppresses_boundary_face() {
        let chunk = Chunk::filled(ChunkCoord::new(0, 0, 0), Material::Stone);
        let quads = mesh_chunk(&chunk, |x, _, _| {
            if x == CHUNK_EDGE as i32 {
                Material::Stone
            } else {
                Material::Air
            }
        });
        assert_eq!(quads.len(), 5);
        assert!(!quads.iter().any(|quad| quad.face == FACE_POS_X));
    }

    #[test]
    fn material_boundaries_are_not_merged() {
        let mut chunk = Chunk::empty(ChunkCoord::new(0, 0, 0));
        for z in 0..CHUNK_EDGE {
            for x in 0..CHUNK_EDGE {
                chunk.set(
                    x,
                    0,
                    z,
                    if (x + z) % 2 == 0 {
                        Material::Stone
                    } else {
                        Material::Dirt
                    },
                );
            }
        }
        let quads = mesh_chunk(&chunk, |_, _, _| Material::Air);
        let top = quads.iter().filter(|quad| quad.face == FACE_POS_Y).count();
        assert_eq!(top, CHUNK_EDGE * CHUNK_EDGE);
    }
}
