//! Word-wide canonical chunk meshing.
//!
//! The authoritative world remains a dense 32-cubed material chunk plus its exact one-voxel
//! shell. This module only accelerates derivation of visible faces: occupancy is transposed into
//! bit columns, opposing cells are culled a word at a time, and material/AO work is performed only
//! for surviving faces. The output contract is exactly [`crate::MeshedChunk`].

use crate::mesh::heightfield_owns_face;
use crate::{
    CHUNK_EDGE, Chunk, EmissiveCluster, Material, MeshedChunk, MeshingEditEnvelope,
    MeshingSurfaceEnvelope, Quad, RenderLayer,
};

const HALO_EDGE: usize = CHUNK_EDGE + 2;
const HALO_PLANE: usize = HALO_EDGE * HALO_EDGE;
const HALO_VOLUME: usize = HALO_PLANE * HALO_EDGE;
const FACE_PLANE: usize = CHUNK_EDGE * CHUNK_EDGE;
const EMISSIVE_BIN_EDGE: usize = 8;
const EMISSIVE_BINS_PER_AXIS: usize = CHUNK_EDGE / EMISSIVE_BIN_EDGE;
const EMISSIVE_BIN_COUNT: usize =
    EMISSIVE_BINS_PER_AXIS * EMISSIVE_BINS_PER_AXIS * EMISSIVE_BINS_PER_AXIS;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct FaceKey {
    material: Material,
    ao: u8,
    surface_owned: bool,
    edit_owned: bool,
}

#[derive(Debug)]
struct KeyPlane {
    key: FaceKey,
    rows: [u32; CHUNK_EDGE],
}

/// Reusable disposable storage for deriving many chunk meshes without allocator churn.
///
/// It never owns world truth and is safe to reuse after any edit or source revision.
pub struct BinaryMeshScratch {
    halo: Vec<u8>,
    opaque_columns: Box<[u64]>,
    translucent_columns: Box<[u64]>,
    face_columns: Box<[u32]>,
    keyed_planes: Vec<Vec<KeyPlane>>,
}

impl Default for BinaryMeshScratch {
    fn default() -> Self {
        Self {
            halo: vec![0; HALO_VOLUME],
            opaque_columns: vec![0; 3 * HALO_PLANE].into_boxed_slice(),
            translucent_columns: vec![0; 3 * HALO_PLANE].into_boxed_slice(),
            face_columns: vec![0; 6 * FACE_PLANE].into_boxed_slice(),
            keyed_planes: (0..CHUNK_EDGE).map(|_| Vec::new()).collect(),
        }
    }
}

impl BinaryMeshScratch {
    pub fn retained_bytes(&self) -> usize {
        size_of::<Self>()
            + self.halo.capacity() * size_of::<u8>()
            + size_of_val(self.opaque_columns.as_ref())
            + size_of_val(self.translucent_columns.as_ref())
            + size_of_val(self.face_columns.as_ref())
            + self.keyed_planes.capacity() * size_of::<Vec<KeyPlane>>()
            + self
                .keyed_planes
                .iter()
                .map(|planes| planes.capacity() * size_of::<KeyPlane>())
                .sum::<usize>()
    }
}

/// Builds a canonical mesh through the word-wide path with one temporary scratch allocation.
pub fn mesh_chunk_binary(
    chunk: &Chunk,
    outside: impl FnMut(i32, i32, i32) -> Material,
) -> MeshedChunk {
    mesh_chunk_binary_with_scratch_and_surface(
        chunk,
        None,
        None,
        outside,
        &mut BinaryMeshScratch::default(),
    )
}

pub fn mesh_chunk_binary_with_surface(
    chunk: &Chunk,
    surface: &MeshingSurfaceEnvelope,
    outside: impl FnMut(i32, i32, i32) -> Material,
) -> MeshedChunk {
    mesh_chunk_binary_with_scratch_and_surface(
        chunk,
        Some(surface),
        None,
        outside,
        &mut BinaryMeshScratch::default(),
    )
}

/// Builds a canonical mesh while reusing caller-owned derivation storage.
pub fn mesh_chunk_binary_with_scratch(
    chunk: &Chunk,
    outside: impl FnMut(i32, i32, i32) -> Material,
    scratch: &mut BinaryMeshScratch,
) -> MeshedChunk {
    mesh_chunk_binary_with_scratch_and_surface(chunk, None, None, outside, scratch)
}

pub fn mesh_chunk_binary_with_scratch_and_surface(
    chunk: &Chunk,
    surface: Option<&MeshingSurfaceEnvelope>,
    edits: Option<&MeshingEditEnvelope>,
    mut outside: impl FnMut(i32, i32, i32) -> Material,
    scratch: &mut BinaryMeshScratch,
) -> MeshedChunk {
    if let Some(surface) = surface {
        assert_eq!(
            surface.coord(),
            chunk.coord(),
            "meshing surface must describe the meshed chunk"
        );
    }
    if let Some(edits) = edits {
        assert_eq!(
            edits.coord(),
            chunk.coord(),
            "meshing edit provenance must describe the meshed chunk"
        );
    }
    scratch.halo.fill(0);
    scratch.opaque_columns.fill(0);
    scratch.translucent_columns.fill(0);
    scratch.face_columns.fill(0);
    for planes in &mut scratch.keyed_planes {
        planes.clear();
    }

    let origin = chunk.coord().world_origin();
    for y in -1..=CHUNK_EDGE as i32 {
        for z in -1..=CHUNK_EDGE as i32 {
            for x in -1..=CHUNK_EDGE as i32 {
                let inside = [x, y, z]
                    .into_iter()
                    .all(|value| value >= 0 && value < CHUNK_EDGE as i32);
                let material = if inside {
                    chunk.get(x as usize, y as usize, z as usize)
                } else {
                    match [
                        origin[0].checked_add(x),
                        origin[1].checked_add(y),
                        origin[2].checked_add(z),
                    ] {
                        [Some(x), Some(y), Some(z)] => outside(x, y, z),
                        _ => Material::Air,
                    }
                };
                let [hx, hy, hz] = [(x + 1) as usize, (y + 1) as usize, (z + 1) as usize];
                let kind = match material.render_layer() {
                    RenderLayer::Empty => 0,
                    RenderLayer::Opaque => 1,
                    RenderLayer::Translucent => 2,
                };
                scratch.halo[halo_index([x, y, z])] = kind;
                if kind == 0 {
                    continue;
                }
                let columns = if kind == 1 {
                    &mut scratch.opaque_columns
                } else {
                    &mut scratch.translucent_columns
                };
                columns[hz + hy * HALO_EDGE] |= 1_u64 << hx;
                columns[HALO_PLANE + hx + hz * HALO_EDGE] |= 1_u64 << hy;
                columns[2 * HALO_PLANE + hx + hy * HALO_EDGE] |= 1_u64 << hz;
            }
        }
    }

    let mut mesh = MeshedChunk::default();
    let mut opaque_edits = Vec::new();
    let mut opaque_volume = Vec::new();
    let mut translucent_edits = Vec::new();
    let mut translucent_volume = Vec::new();
    append_emissive_clusters(chunk, &scratch.halo, &mut mesh.emissive_clusters);
    build_visible_face_columns(scratch);

    for face in 0..6 {
        for v in 0..CHUNK_EDGE {
            for u in 0..CHUNK_EDGE {
                let mut slices = scratch.face_columns[face * FACE_PLANE + u + v * CHUNK_EDGE];
                while slices != 0 {
                    let slice = slices.trailing_zeros() as usize;
                    slices &= slices - 1;
                    let (axis, u_axis, v_axis, _) = face_axes(face as u8);
                    let local = compose(axis, u_axis, v_axis, slice, u, v);
                    let (_, _, _, step) = face_axes(face as u8);
                    let mut neighbor = local.map(|value| value as i32);
                    neighbor[axis] += step;
                    let material = chunk.get(local[0], local[1], local[2]);
                    let key = FaceKey {
                        material,
                        ao: face_ao(
                            local,
                            axis,
                            u_axis,
                            v_axis,
                            if face & 1 == 0 { 1 } else { -1 },
                            &scratch.halo,
                        ),
                        surface_owned: surface.is_some_and(|surface| {
                            heightfield_owns_face(
                                surface, origin, local, neighbor, material, face as u8,
                            )
                        }),
                        edit_owned: edits.is_some_and(|edits| {
                            edits.sample_local(local[0] as i32, local[1] as i32, local[2] as i32)
                                || edits.sample_local(neighbor[0], neighbor[1], neighbor[2])
                        }),
                    };
                    let planes = &mut scratch.keyed_planes[slice];
                    let plane_index = planes
                        .iter()
                        .position(|plane| plane.key == key)
                        .unwrap_or_else(|| {
                            planes.push(KeyPlane {
                                key,
                                rows: [0; CHUNK_EDGE],
                            });
                            planes.len() - 1
                        });
                    planes[plane_index].rows[v] |= 1_u32 << u;
                }
            }
        }

        for slice in 0..CHUNK_EDGE {
            for plane in &mut scratch.keyed_planes[slice] {
                let mut partitions = MeshPartitions {
                    mesh: &mut mesh,
                    opaque_edits: &mut opaque_edits,
                    opaque_volume: &mut opaque_volume,
                    translucent_edits: &mut translucent_edits,
                    translucent_volume: &mut translucent_volume,
                };
                append_binary_plane(
                    &mut partitions,
                    face as u8,
                    slice,
                    plane.key,
                    &mut plane.rows,
                );
            }
            scratch.keyed_planes[slice].clear();
        }
    }
    mesh.opaque_surface_quads = mesh.opaque.len() as u32;
    mesh.translucent_surface_quads = mesh.translucent.len() as u32;
    mesh.opaque_edit_quads = opaque_edits.len() as u32;
    mesh.translucent_edit_quads = translucent_edits.len() as u32;
    mesh.opaque.extend(opaque_edits);
    mesh.opaque.extend(opaque_volume);
    mesh.translucent.extend(translucent_edits);
    mesh.translucent.extend(translucent_volume);
    mesh
}

struct MeshPartitions<'a> {
    mesh: &'a mut MeshedChunk,
    opaque_edits: &'a mut Vec<Quad>,
    opaque_volume: &'a mut Vec<Quad>,
    translucent_edits: &'a mut Vec<Quad>,
    translucent_volume: &'a mut Vec<Quad>,
}

fn build_visible_face_columns(scratch: &mut BinaryMeshScratch) {
    const INTERIOR_BITS: u64 = ((1_u64 << CHUNK_EDGE) - 1) << 1;
    for face in 0..6 {
        let axis = face / 2;
        for v in 0..CHUNK_EDGE {
            for u in 0..CHUNK_EDGE {
                let column_index = (u + 1) + (v + 1) * HALO_EDGE;
                let column_index = axis * HALO_PLANE + column_index;
                let opaque = scratch.opaque_columns[column_index];
                let translucent = scratch.translucent_columns[column_index];
                let renderable = opaque | translucent;
                let (neighbor_opaque, neighbor_renderable) = if face & 1 == 0 {
                    (opaque >> 1, renderable >> 1)
                } else {
                    (opaque << 1, renderable << 1)
                };
                // Opaque faces remain visible through translucent neighbors. Translucent faces are
                // only emitted against air, avoiding duplicate internal alpha interfaces.
                let visible = (opaque & !neighbor_opaque) | (translucent & !neighbor_renderable);
                scratch.face_columns[face * FACE_PLANE + u + v * CHUNK_EDGE] =
                    ((visible & INTERIOR_BITS) >> 1) as u32;
            }
        }
    }
}

fn append_binary_plane(
    output: &mut MeshPartitions<'_>,
    face: u8,
    slice: usize,
    key: FaceKey,
    rows: &mut [u32; CHUNK_EDGE],
) {
    let (axis, u_axis, v_axis, _) = face_axes(face);
    for v in 0..CHUNK_EDGE {
        while rows[v] != 0 {
            let u = rows[v].trailing_zeros() as usize;
            let width = (rows[v] >> u).trailing_ones() as usize;
            let run = if width == u32::BITS as usize {
                u32::MAX
            } else {
                (1_u32 << width) - 1
            };
            let span = run << u;
            rows[v] &= !span;
            let mut height = 1;
            while v + height < CHUNK_EDGE && rows[v + height] & span == span {
                rows[v + height] &= !span;
                height += 1;
            }
            let local = compose(axis, u_axis, v_axis, slice, u, v);
            let quad = Quad {
                origin: local.map(|value| value as u8),
                face,
                extent: [width as u8, height as u8],
                material: key.material.id(),
                ao: key.ao,
                _pad: 0,
            };
            match key.material.render_layer() {
                RenderLayer::Opaque if key.edit_owned => output.opaque_edits.push(quad),
                RenderLayer::Opaque if key.surface_owned => output.mesh.opaque.push(quad),
                RenderLayer::Opaque => output.opaque_volume.push(quad),
                RenderLayer::Translucent if key.edit_owned => {
                    output.translucent_edits.push(quad);
                }
                RenderLayer::Translucent if key.surface_owned => output.mesh.translucent.push(quad),
                RenderLayer::Translucent => output.translucent_volume.push(quad),
                RenderLayer::Empty => unreachable!("visible face cannot belong to air"),
            }
        }
    }
}

fn append_emissive_clusters(chunk: &Chunk, halo: &[u8], output: &mut Vec<EmissiveCluster>) {
    let mut counts = [0_u16; Material::ALL.len() * EMISSIVE_BIN_COUNT];
    let mut sums = [[0_u32; 3]; Material::ALL.len() * EMISSIVE_BIN_COUNT];
    for y in 0..CHUNK_EDGE {
        for z in 0..CHUNK_EDGE {
            for x in 0..CHUNK_EDGE {
                let material = chunk.get(x, y, z);
                if material.emission().is_none()
                    || !emitter_is_exposed(halo, [x as i32, y as i32, z as i32])
                {
                    continue;
                }
                let bin = x / EMISSIVE_BIN_EDGE
                    + z / EMISSIVE_BIN_EDGE * EMISSIVE_BINS_PER_AXIS
                    + y / EMISSIVE_BIN_EDGE * EMISSIVE_BINS_PER_AXIS * EMISSIVE_BINS_PER_AXIS;
                let index = usize::from(material.id()) * EMISSIVE_BIN_COUNT + bin;
                counts[index] = counts[index].saturating_add(1);
                for (axis, value) in [x, y, z].into_iter().enumerate() {
                    sums[index][axis] = sums[index][axis]
                        .saturating_add((value as u32).saturating_mul(2).saturating_add(1));
                }
            }
        }
    }
    for material in Material::ALL {
        for bin in 0..EMISSIVE_BIN_COUNT {
            let index = usize::from(material.id()) * EMISSIVE_BIN_COUNT + bin;
            if counts[index] > 0 {
                output.push(EmissiveCluster {
                    position_half_voxel_sum: sums[index],
                    voxel_count: counts[index],
                    material: material.id(),
                });
            }
        }
    }
}

fn emitter_is_exposed(halo: &[u8], local: [i32; 3]) -> bool {
    [
        [-1, 0, 0],
        [1, 0, 0],
        [0, -1, 0],
        [0, 1, 0],
        [0, 0, -1],
        [0, 0, 1],
    ]
    .into_iter()
    .any(|offset| {
        halo[halo_index([
            local[0] + offset[0],
            local[1] + offset[1],
            local[2] + offset[2],
        ])] != 1
    })
}

fn face_ao(
    local: [usize; 3],
    axis: usize,
    u_axis: usize,
    v_axis: usize,
    step: i32,
    halo: &[u8],
) -> u8 {
    let mut base = local.map(|value| value as i32);
    base[axis] += step;
    let mut packed = 0;
    for corner in 0..4 {
        let high_u = corner == 1 || corner == 2;
        let high_v = corner >= 2;
        let du = if high_u { 1 } else { -1 };
        let dv = if high_v { 1 } else { -1 };
        let mut side_u = base;
        side_u[u_axis] += du;
        let mut side_v = base;
        side_v[v_axis] += dv;
        let mut diagonal = side_u;
        diagonal[v_axis] += dv;
        let side_u = halo[halo_index(side_u)] == 1;
        let side_v = halo[halo_index(side_v)] == 1;
        let diagonal = halo[halo_index(diagonal)] == 1;
        let ao = if side_u && side_v {
            0
        } else {
            3 - u8::from(side_u) - u8::from(side_v) - u8::from(diagonal)
        };
        packed |= ao << (corner * 2);
    }
    packed
}

fn halo_index(local: [i32; 3]) -> usize {
    (local[0] + 1) as usize
        + (local[2] + 1) as usize * HALO_EDGE
        + (local[1] + 1) as usize * HALO_PLANE
}

fn face_axes(face: u8) -> (usize, usize, usize, i32) {
    match face {
        0 => (0, 2, 1, 1),
        1 => (0, 2, 1, -1),
        2 => (1, 0, 2, 1),
        3 => (1, 0, 2, -1),
        4 => (2, 0, 1, 1),
        5 => (2, 0, 1, -1),
        _ => unreachable!("face is generated from 0..6"),
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
    use crate::mesh::mesh_chunk_with_ownership;
    use crate::{
        ChunkCoord, Generator, MeshingEditEnvelope, MeshingSurfaceColumn, MeshingSurfaceEnvelope,
        mesh_chunk,
    };

    #[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
    struct UnitFace {
        origin: [u8; 3],
        face: u8,
        material: u16,
        ao: u8,
    }

    fn unit_faces(quads: &[Quad]) -> Vec<UnitFace> {
        let mut output = Vec::new();
        for quad in quads {
            let (_, u_axis, v_axis, _) = face_axes(quad.face);
            for v in 0..quad.extent[1] {
                for u in 0..quad.extent[0] {
                    let mut origin = quad.origin;
                    origin[u_axis] += u;
                    origin[v_axis] += v;
                    output.push(UnitFace {
                        origin,
                        face: quad.face,
                        material: quad.material,
                        ao: quad.ao,
                    });
                }
            }
        }
        output.sort_unstable();
        output
    }

    fn assert_equivalent(chunk: &Chunk, outside: impl Fn(i32, i32, i32) -> Material + Copy) {
        let scalar = mesh_chunk(chunk, outside);
        let binary = mesh_chunk_binary(chunk, outside);
        assert_eq!(unit_faces(&binary.opaque), unit_faces(&scalar.opaque));
        assert_eq!(
            unit_faces(&binary.translucent),
            unit_faces(&scalar.translucent)
        );
        assert_eq!(binary.emissive_clusters, scalar.emissive_clusters);
    }

    fn assert_owned_equivalent(
        chunk: &Chunk,
        surface: &MeshingSurfaceEnvelope,
        edits: Option<&MeshingEditEnvelope>,
        outside: impl Fn(i32, i32, i32) -> Material + Copy,
    ) {
        let scalar = mesh_chunk_with_ownership(chunk, Some(surface), edits, outside);
        let binary = mesh_chunk_binary_with_scratch_and_surface(
            chunk,
            Some(surface),
            edits,
            outside,
            &mut BinaryMeshScratch::default(),
        );
        assert_eq!(binary.opaque_surface_quads, scalar.opaque_surface_quads);
        assert_eq!(binary.opaque_edit_quads, scalar.opaque_edit_quads);
        assert_eq!(
            binary.translucent_surface_quads,
            scalar.translucent_surface_quads
        );
        assert_eq!(binary.translucent_edit_quads, scalar.translucent_edit_quads);
        assert_eq!(
            unit_faces(&binary.opaque[..binary.opaque_surface_quads as usize]),
            unit_faces(&scalar.opaque[..scalar.opaque_surface_quads as usize])
        );
        assert_eq!(
            unit_faces(&binary.opaque[binary.opaque_surface_quads as usize..]),
            unit_faces(&scalar.opaque[scalar.opaque_surface_quads as usize..])
        );
        assert_eq!(
            unit_faces(&binary.translucent[..binary.translucent_surface_quads as usize]),
            unit_faces(&scalar.translucent[..scalar.translucent_surface_quads as usize])
        );
        assert_eq!(
            unit_faces(&binary.translucent[binary.translucent_surface_quads as usize..]),
            unit_faces(&scalar.translucent[scalar.translucent_surface_quads as usize..])
        );
    }

    #[test]
    fn binary_mesh_matches_scalar_for_uniform_and_mixed_layers() {
        assert_equivalent(&Chunk::empty(ChunkCoord::new(0, 0, 0)), |_, _, _| {
            Material::Air
        });
        assert_equivalent(
            &Chunk::filled(ChunkCoord::new(-1, 2, -3), Material::Stone),
            |_, _, _| Material::Air,
        );

        let mut mixed = Chunk::empty(ChunkCoord::new(-2, 1, 3));
        for y in 3..21 {
            for z in 5..27 {
                for x in 2..29 {
                    let material = match (x + y * 3 + z * 5) % 11 {
                        0 => Material::Water,
                        1 => Material::GlowCrystal,
                        2 | 3 => Material::Basalt,
                        _ => Material::Stone,
                    };
                    mixed.set(x, y, z, material);
                }
            }
        }
        assert_equivalent(&mixed, |_, _, _| Material::Air);
    }

    #[test]
    fn binary_mesh_matches_scalar_surface_partition_across_edits_and_layers() {
        let coord = ChunkCoord::new(-2, 1, 3);
        let origin_y = coord.world_origin()[1];
        let surface = MeshingSurfaceEnvelope::from_columns(
            coord,
            vec![
                MeshingSurfaceColumn {
                    valid: true,
                    ground_plane: origin_y + 10,
                    water_plane: Some(origin_y + 14),
                };
                MeshingSurfaceEnvelope::COLUMN_COUNT
            ],
        )
        .unwrap();
        let mut chunk = Chunk::empty(coord);
        for y in 0..10 {
            for z in 0..CHUNK_EDGE {
                for x in 0..CHUNK_EDGE {
                    chunk.set(x, y, z, Material::Stone);
                }
            }
        }
        for y in 10..14 {
            for z in 0..CHUNK_EDGE {
                for x in 0..CHUNK_EDGE {
                    chunk.set(x, y, z, Material::Water);
                }
            }
        }
        chunk.set(7, 4, 8, Material::Air);
        chunk.set(9, 17, 10, Material::Basalt);
        let origin = coord.world_origin();
        let edits = MeshingEditEnvelope::from_sampler(coord, |x, y, z| {
            [x, y, z] == [origin[0] + 7, origin[1] + 4, origin[2] + 8]
                || [x, y, z] == [origin[0] + 9, origin[1] + 17, origin[2] + 10]
        })
        .unwrap();
        assert_owned_equivalent(&chunk, &surface, Some(&edits), |_, y, _| {
            if y < origin_y + 10 {
                Material::Stone
            } else if y < origin_y + 14 {
                Material::Water
            } else {
                Material::Air
            }
        });
    }

    #[test]
    fn binary_mesh_matches_scalar_for_procedural_chunks_and_neighbors() {
        let generator = Generator::new(0x5eed_cafe_d15c_a11e);
        for coord in [
            ChunkCoord::new(0, 0, 0),
            ChunkCoord::new(5, 4, -7),
            ChunkCoord::new(-11, 3, 9),
            ChunkCoord::new(31, -2, -17),
        ] {
            let chunk = generator.generate_chunk(coord);
            assert_equivalent(&chunk, |x, y, z| generator.sample(x, y, z));
        }
    }

    #[test]
    fn reusable_scratch_does_not_retain_previous_chunk_faces() {
        let mut scratch = BinaryMeshScratch::default();
        let solid = Chunk::filled(ChunkCoord::new(0, 0, 0), Material::Stone);
        let first = mesh_chunk_binary_with_scratch(&solid, |_, _, _| Material::Air, &mut scratch);
        assert_eq!(first.opaque.len(), 6);
        let empty = Chunk::empty(ChunkCoord::new(0, 0, 0));
        let second = mesh_chunk_binary_with_scratch(&empty, |_, _, _| Material::Air, &mut scratch);
        assert!(second.is_empty());
        assert!(scratch.retained_bytes() > 0);
    }
}
