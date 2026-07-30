use crate::{CHUNK_EDGE, Chunk, Material, MeshingSurfaceEnvelope, RenderLayer};
use bytemuck::{Pod, Zeroable};

pub const FACE_POS_X: u8 = 0;
pub const FACE_NEG_X: u8 = 1;
pub const FACE_POS_Y: u8 = 2;
pub const FACE_NEG_Y: u8 = 3;
pub const FACE_POS_Z: u8 = 4;
pub const FACE_NEG_Z: u8 = 5;
const EMISSIVE_BIN_EDGE: usize = 8;
const EMISSIVE_BINS_PER_AXIS: usize = CHUNK_EDGE / EMISSIVE_BIN_EDGE;
const EMISSIVE_BIN_COUNT: usize =
    EMISSIVE_BINS_PER_AXIS * EMISSIVE_BINS_PER_AXIS * EMISSIVE_BINS_PER_AXIS;

/// Compact durable mesher output. The renderer expands each record into six vertices on the GPU.
#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, Pod, Zeroable)]
pub struct Quad {
    pub origin: [u8; 3],
    pub face: u8,
    pub extent: [u8; 2],
    pub material: u16,
    /// Four 2-bit corner values in (0,0), (1,0), (1,1), (0,1) order; 3 is unoccluded.
    pub ao: u8,
    pub _pad: u8,
}

const _: () = assert!(size_of::<Quad>() == 10);

/// One material-cluster light derived from canonical voxels during meshing. Position sums use
/// half-voxel units (`2 * local + 1`), retaining exact deterministic centroids without floats.
#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, Pod, Zeroable)]
pub struct EmissiveCluster {
    pub position_half_voxel_sum: [u32; 3],
    pub voxel_count: u16,
    pub material: u16,
}

const _: () = assert!(size_of::<EmissiveCluster>() == 16);

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MeshedChunk {
    /// Prefix length in `opaque` represented by the source heightfield envelope.
    pub opaque_surface_quads: u32,
    pub opaque: Vec<Quad>,
    /// Prefix length in `translucent` represented by the source heightfield envelope.
    pub translucent_surface_quads: u32,
    pub translucent: Vec<Quad>,
    pub emissive_clusters: Vec<EmissiveCluster>,
}

impl MeshedChunk {
    pub fn is_empty(&self) -> bool {
        self.opaque.is_empty() && self.translucent.is_empty()
    }

    pub fn retained_bytes(&self) -> usize {
        size_of::<Self>()
            + self.opaque.capacity() * size_of::<Quad>()
            + self.translucent.capacity() * size_of::<Quad>()
            + self.emissive_clusters.capacity() * size_of::<EmissiveCluster>()
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct FaceKey {
    material: Material,
    ao: u8,
    surface_owned: bool,
}

/// Greedily merges coplanar visible faces with the same material. `outside` samples world coordinates
/// beyond this chunk, preventing hidden seam faces when neighboring chunks are resident or generated.
pub fn mesh_chunk(
    chunk: &Chunk,
    outside: impl FnMut(i32, i32, i32) -> Material,
) -> MeshedChunk {
    mesh_chunk_with_surface(chunk, None, outside)
}

/// Greedily meshes a chunk while retaining an exact partition between faces represented by the
/// source heightfield envelope and canonical volume-only faces.
///
/// The partition is semantic, not inferred from a one-voxel air scan: base tops and cliff risers
/// are heightfield-owned, while caves, undersides, features, and edited departures remain volume.
pub fn mesh_chunk_with_surface(
    chunk: &Chunk,
    surface: Option<&MeshingSurfaceEnvelope>,
    mut outside: impl FnMut(i32, i32, i32) -> Material,
) -> MeshedChunk {
    if let Some(surface) = surface {
        assert_eq!(
            surface.coord(),
            chunk.coord(),
            "meshing surface must describe the meshed chunk"
        );
    }
    let mut mesh = MeshedChunk::default();
    let mut opaque_volume = Vec::new();
    let mut translucent_volume = Vec::new();
    let origin = chunk.coord().world_origin();
    let occupancy = build_occupancy_halo(chunk, origin, &mut outside);
    let mut emission_counts = [0u16; Material::ALL.len() * EMISSIVE_BIN_COUNT];
    let mut emission_sums = [[0u32; 3]; Material::ALL.len() * EMISSIVE_BIN_COUNT];
    for y in 0..CHUNK_EDGE {
        for z in 0..CHUNK_EDGE {
            for x in 0..CHUNK_EDGE {
                let material = chunk.get(x, y, z);
                if material.emission().is_none() || !emitter_is_exposed(&occupancy, [x, y, z]) {
                    continue;
                }
                let bin = x / EMISSIVE_BIN_EDGE
                    + z / EMISSIVE_BIN_EDGE * EMISSIVE_BINS_PER_AXIS
                    + y / EMISSIVE_BIN_EDGE * EMISSIVE_BINS_PER_AXIS * EMISSIVE_BINS_PER_AXIS;
                let index = usize::from(material.id()) * EMISSIVE_BIN_COUNT + bin;
                emission_counts[index] = emission_counts[index].saturating_add(1);
                for (axis, value) in [x, y, z].into_iter().enumerate() {
                    emission_sums[index][axis] = emission_sums[index][axis]
                        .saturating_add((value as u32).saturating_mul(2).saturating_add(1));
                }
            }
        }
    }
    for material in Material::ALL {
        for bin in 0..EMISSIVE_BIN_COUNT {
            let index = usize::from(material.id()) * EMISSIVE_BIN_COUNT + bin;
            if emission_counts[index] > 0 {
                mesh.emissive_clusters.push(EmissiveCluster {
                    position_half_voxel_sum: emission_sums[index],
                    voxel_count: emission_counts[index],
                    material: material.id(),
                });
            }
        }
    }
    let mut mask = vec![FaceKey::default(); CHUNK_EDGE * CHUNK_EDGE];
    for face in 0..6 {
        let (axis, u_axis, v_axis, step) = face_axes(face);
        for slice in 0..CHUNK_EDGE {
            for v in 0..CHUNK_EDGE {
                for u in 0..CHUNK_EDGE {
                    let local = compose(axis, u_axis, v_axis, slice, u, v);
                    let material = chunk.get(local[0], local[1], local[2]);
                    if !material.is_renderable() {
                        mask[u + v * CHUNK_EDGE] = FaceKey::default();
                        continue;
                    }
                    let mut neighbor = [local[0] as i32, local[1] as i32, local[2] as i32];
                    neighbor[axis] += step;
                    let neighbor_kind = halo_kind(&occupancy, neighbor);
                    mask[u + v * CHUNK_EDGE] = if !face_visible(material, neighbor_kind) {
                        FaceKey::default()
                    } else {
                        FaceKey {
                            material,
                            ao: face_ao(local, axis, u_axis, v_axis, step, &occupancy),
                            surface_owned: surface.is_some_and(|surface| {
                                heightfield_owns_face(
                                    surface, origin, local, neighbor, material, face,
                                )
                            }),
                        }
                    };
                }
            }

            let mut v = 0;
            while v < CHUNK_EDGE {
                let mut u = 0;
                while u < CHUNK_EDGE {
                    let key = mask[u + v * CHUNK_EDGE];
                    if !key.material.is_renderable() {
                        u += 1;
                        continue;
                    }
                    let mut width = 1;
                    while u + width < CHUNK_EDGE && mask[u + width + v * CHUNK_EDGE] == key {
                        width += 1;
                    }
                    let mut height = 1;
                    'height: while v + height < CHUNK_EDGE {
                        for offset in 0..width {
                            if mask[u + offset + (v + height) * CHUNK_EDGE] != key {
                                break 'height;
                            }
                        }
                        height += 1;
                    }
                    for clear_v in v..v + height {
                        for clear_u in u..u + width {
                            mask[clear_u + clear_v * CHUNK_EDGE] = FaceKey::default();
                        }
                    }
                    let local = compose(axis, u_axis, v_axis, slice, u, v);
                    let quad = Quad {
                        origin: [local[0] as u8, local[1] as u8, local[2] as u8],
                        face,
                        extent: [width as u8, height as u8],
                        material: key.material.id(),
                        ao: key.ao,
                        _pad: 0,
                    };
                    match key.material.render_layer() {
                        RenderLayer::Opaque if key.surface_owned => mesh.opaque.push(quad),
                        RenderLayer::Opaque => opaque_volume.push(quad),
                        RenderLayer::Translucent if key.surface_owned => {
                            mesh.translucent.push(quad);
                        }
                        RenderLayer::Translucent => translucent_volume.push(quad),
                        RenderLayer::Empty => {}
                    }
                    u += width;
                }
                v += 1;
            }
        }
    }
    mesh.opaque_surface_quads = mesh.opaque.len() as u32;
    mesh.translucent_surface_quads = mesh.translucent.len() as u32;
    mesh.opaque.extend(opaque_volume);
    mesh.translucent.extend(translucent_volume);
    mesh
}

pub(crate) fn heightfield_owns_face(
    surface: &MeshingSurfaceEnvelope,
    world_origin: [i32; 3],
    local: [usize; 3],
    neighbor: [i32; 3],
    material: Material,
    face: u8,
) -> bool {
    let current_y = world_origin[1].saturating_add(local[1] as i32);
    if material == Material::Water {
        // Level-zero virtual water follows the same lower-X/Z voxel-cell convention as ground.
        // Only top faces exist in that representation; water sides and bottoms remain canonical.
        return face == FACE_POS_Y
            && surface
                .sample_local(local[0] as i32, local[2] as i32)
                .filter(|column| column.valid)
                .and_then(|column| column.water_plane)
                == current_y.checked_add(1);
    }
    if material.render_layer() != RenderLayer::Opaque {
        return false;
    }
    let Some(current) = surface
        .sample_local(local[0] as i32, local[2] as i32)
        .filter(|column| column.valid)
    else {
        return false;
    };
    let Some(neighbor_column) = surface
        .sample_local(neighbor[0], neighbor[2])
        .filter(|column| column.valid)
    else {
        return false;
    };
    let neighbor_y = world_origin[1].saturating_add(neighbor[1]);
    current_y < current.ground_plane && neighbor_y >= neighbor_column.ground_plane
}

fn emitter_is_exposed(occupancy: &[u8], local: [usize; 3]) -> bool {
    let local = local.map(|value| value as i32);
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
        halo_kind(
            occupancy,
            [
                local[0] + offset[0],
                local[1] + offset[1],
                local[2] + offset[2],
            ],
        ) != 1
    })
}

fn build_occupancy_halo(
    chunk: &Chunk,
    origin: [i32; 3],
    outside: &mut impl FnMut(i32, i32, i32) -> Material,
) -> Vec<u8> {
    const HALO_EDGE: usize = CHUNK_EDGE + 2;
    let mut occupancy = vec![0; HALO_EDGE * HALO_EDGE * HALO_EDGE];
    for y in -1..=CHUNK_EDGE as i32 {
        for z in -1..=CHUNK_EDGE as i32 {
            for x in -1..=CHUNK_EDGE as i32 {
                let inside = [x, y, z]
                    .iter()
                    .all(|value| *value >= 0 && *value < CHUNK_EDGE as i32);
                let material = if inside {
                    chunk.get(x as usize, y as usize, z as usize)
                } else {
                    let world = [
                        origin[0].checked_add(x),
                        origin[1].checked_add(y),
                        origin[2].checked_add(z),
                    ];
                    match world {
                        [Some(x), Some(y), Some(z)] => outside(x, y, z),
                        // The canonical i32 voxel grid is finite. A missing halo cell beyond that
                        // domain is empty space, not a wrapped sample from the opposite boundary.
                        _ => Material::Air,
                    }
                };
                let index = (x + 1) as usize
                    + (z + 1) as usize * HALO_EDGE
                    + (y + 1) as usize * HALO_EDGE * HALO_EDGE;
                occupancy[index] = match material.render_layer() {
                    RenderLayer::Empty => 0,
                    RenderLayer::Opaque => 1,
                    RenderLayer::Translucent => 2,
                };
            }
        }
    }
    occupancy
}

fn face_visible(material: Material, neighbor_kind: u8) -> bool {
    if material == Material::Water {
        neighbor_kind == 0
    } else {
        neighbor_kind != 1
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "face basis is explicit for deterministic AO sampling"
)]
fn face_ao(
    local: [usize; 3],
    axis: usize,
    u_axis: usize,
    v_axis: usize,
    step: i32,
    occupancy: &[u8],
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
        let side_u = halo_solid(occupancy, side_u);
        let side_v = halo_solid(occupancy, side_v);
        let diagonal = halo_solid(occupancy, diagonal);
        let ao = if side_u && side_v {
            0
        } else {
            3 - u8::from(side_u) - u8::from(side_v) - u8::from(diagonal)
        };
        packed |= ao << (corner * 2);
    }
    packed
}

fn halo_kind(occupancy: &[u8], local: [i32; 3]) -> u8 {
    const HALO_EDGE: usize = CHUNK_EDGE + 2;
    let index = (local[0] + 1) as usize
        + (local[2] + 1) as usize * HALO_EDGE
        + (local[1] + 1) as usize * HALO_EDGE * HALO_EDGE;
    occupancy[index]
}

fn halo_solid(occupancy: &[u8], local: [i32; 3]) -> bool {
    halo_kind(occupancy, local) == 1
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
    use crate::{ChunkCoord, MeshingSurfaceColumn, VoxelCoord};
    use std::collections::BTreeSet;

    fn flat_surface(
        coord: ChunkCoord,
        ground_plane: i32,
        water_plane: Option<i32>,
    ) -> MeshingSurfaceEnvelope {
        MeshingSurfaceEnvelope::from_columns(
            coord,
            vec![
                MeshingSurfaceColumn {
                    valid: true,
                    ground_plane,
                    water_plane,
                };
                MeshingSurfaceEnvelope::COLUMN_COUNT
            ],
        )
        .unwrap()
    }

    fn fill_below(chunk: &mut Chunk, plane: impl Fn(usize, usize) -> i32) {
        let origin_y = chunk.coord().world_origin()[1];
        for y in 0..CHUNK_EDGE {
            for z in 0..CHUNK_EDGE {
                for x in 0..CHUNK_EDGE {
                    if origin_y + (y as i32) < plane(x, z) {
                        chunk.set(x, y, z, Material::Stone);
                    }
                }
            }
        }
    }

    #[test]
    fn filled_chunk_merges_to_six_quads() {
        let chunk = Chunk::filled(ChunkCoord::new(0, 0, 0), Material::Stone);
        let mesh = mesh_chunk(&chunk, |_, _, _| Material::Air);
        let quads = mesh.opaque;
        assert_eq!(quads.len(), 6);
        assert!(quads.iter().all(|quad| quad.extent == [32, 32]));
        assert!(quads.iter().all(|quad| quad.ao == 0xff));
    }

    #[test]
    fn mesher_requests_the_exact_deduplicated_one_voxel_shell() {
        let coord = ChunkCoord::new(-2, 3, 4);
        let origin = coord.world_origin();
        let mut sampled = BTreeSet::new();
        let _ = mesh_chunk(&Chunk::empty(coord), |x, y, z| {
            assert!(
                sampled.insert([x, y, z]),
                "mesher sampled one halo voxel twice"
            );
            Material::Air
        });

        let mut expected = BTreeSet::new();
        for y in -1..=CHUNK_EDGE as i32 {
            for z in -1..=CHUNK_EDGE as i32 {
                for x in -1..=CHUNK_EDGE as i32 {
                    let inside = [x, y, z]
                        .into_iter()
                        .all(|value| value >= 0 && value < CHUNK_EDGE as i32);
                    if !inside {
                        expected.insert([origin[0] + x, origin[1] + y, origin[2] + z]);
                    }
                }
            }
        }

        assert_eq!(sampled, expected);
        assert_eq!(sampled.len(), crate::MESHING_HALO_VOXELS);
        assert_eq!(sampled.len(), 6_536);
    }

    #[test]
    fn retained_bytes_include_both_quad_allocations() {
        let chunk = Chunk::filled(ChunkCoord::new(0, 0, 0), Material::Water);
        let mesh = mesh_chunk(&chunk, |_, _, _| Material::Air);
        assert!(mesh.retained_bytes() >= size_of::<MeshedChunk>() + 6 * size_of::<Quad>());
    }

    #[test]
    fn emissive_voxels_form_deterministic_exposed_spatial_clusters() {
        let mut chunk = Chunk::empty(ChunkCoord::new(-3, 2, 7));
        chunk.set(2, 4, 6, Material::GlowCrystal);
        chunk.set(6, 6, 6, Material::GlowCrystal);
        chunk.set(12, 12, 12, Material::Basalt);
        let mesh = mesh_chunk(&chunk, |_, _, _| Material::Air);
        assert_eq!(
            mesh.emissive_clusters,
            vec![EmissiveCluster {
                position_half_voxel_sum: [18, 22, 26],
                voxel_count: 2,
                material: Material::GlowCrystal.id(),
            }]
        );

        chunk.set(2, 4, 6, Material::Air);
        let edited = mesh_chunk(&chunk, |_, _, _| Material::Air);
        assert_eq!(
            edited.emissive_clusters[0].position_half_voxel_sum,
            [13, 13, 13]
        );
        assert_eq!(edited.emissive_clusters[0].voxel_count, 1);

        let mut buried = Chunk::filled(ChunkCoord::new(0, 0, 0), Material::Basalt);
        buried.set(16, 16, 16, Material::GlowCrystal);
        assert!(
            mesh_chunk(&buried, |_, _, _| Material::Basalt)
                .emissive_clusters
                .is_empty()
        );
    }

    #[test]
    fn solid_neighbor_suppresses_boundary_face() {
        let chunk = Chunk::filled(ChunkCoord::new(0, 0, 0), Material::Stone);
        let mesh = mesh_chunk(&chunk, |x, _, _| {
            if x == CHUNK_EDGE as i32 {
                Material::Stone
            } else {
                Material::Air
            }
        });
        // Neighbor occupancy also changes AO along the four adjoining faces, conservatively
        // splitting their merge keys; the hidden positive-X face itself must still be absent.
        assert!(mesh.opaque.len() >= 5);
        assert!(!mesh.opaque.iter().any(|quad| quad.face == FACE_POS_X));
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
        let mesh = mesh_chunk(&chunk, |_, _, _| Material::Air);
        let top = mesh
            .opaque
            .iter()
            .filter(|quad| quad.face == FACE_POS_Y)
            .count();
        assert_eq!(top, CHUNK_EDGE * CHUNK_EDGE);
    }

    #[test]
    fn water_merges_as_renderable_non_occluding_geometry() {
        let chunk = Chunk::filled(ChunkCoord::new(0, 0, 0), Material::Water);
        let mesh = mesh_chunk(&chunk, |_, _, _| Material::Air);
        assert!(mesh.opaque.is_empty());
        assert_eq!(mesh.translucent.len(), 6);
        assert!(
            mesh.translucent
                .iter()
                .all(|quad| quad.material == Material::Water.id())
        );
        assert!(mesh.translucent.iter().all(|quad| quad.extent == [32, 32]));
    }

    #[test]
    fn source_surface_partition_keeps_caves_features_and_remote_ceilings_canonical() {
        let coord = ChunkCoord::new(0, 0, 0);
        let surface = flat_surface(coord, 10, None);
        let mut chunk = Chunk::empty(coord);
        fill_below(&mut chunk, |_, _| 10);
        chunk.set(8, 5, 8, Material::Air);
        chunk.set(12, 14, 12, Material::Stone);
        let mesh = mesh_chunk_with_surface(&chunk, Some(&surface), |_, y, _| {
            if y < 10 {
                Material::Stone
            } else {
                Material::Air
            }
        });
        assert!(mesh.opaque_surface_quads > 0);
        assert!(
            mesh.opaque[..mesh.opaque_surface_quads as usize]
                .iter()
                .all(|quad| quad.face == FACE_POS_Y),
            "the flat source envelope owns only its ground tops"
        );
        let volume = &mesh.opaque[mesh.opaque_surface_quads as usize..];
        assert!(
            volume.iter().any(|quad| quad.origin[1] <= 6),
            "the excavated cave must remain canonical volume"
        );
        assert!(
            volume.iter().any(|quad| quad.origin[1] >= 14),
            "the floating feature must remain canonical volume"
        );

        let remote_coord = ChunkCoord::new(0, 2, 0);
        let remote_surface = flat_surface(remote_coord, 10, None);
        let mut remote = Chunk::empty(remote_coord);
        remote.set(9, 12, 9, Material::Stone);
        let remote_mesh =
            mesh_chunk_with_surface(&remote, Some(&remote_surface), |_, _, _| Material::Air);
        assert_eq!(remote_mesh.opaque_surface_quads, 0);
        assert_eq!(remote_mesh.opaque.len(), 6);
    }

    #[test]
    fn source_surface_partition_owns_ground_cliff_risers_and_only_water_tops() {
        let coord = ChunkCoord::new(0, 0, 0);
        let mut columns = Vec::with_capacity(MeshingSurfaceEnvelope::COLUMN_COUNT);
        for _z in -1..=CHUNK_EDGE as i32 {
            for x in -1..=CHUNK_EDGE as i32 {
                columns.push(MeshingSurfaceColumn {
                    valid: true,
                    ground_plane: if x < 16 { 10 } else { 5 },
                    water_plane: None,
                });
            }
        }
        let surface = MeshingSurfaceEnvelope::from_columns(coord, columns).unwrap();
        let mut chunk = Chunk::empty(coord);
        fill_below(&mut chunk, |x, _| if x < 16 { 10 } else { 5 });
        let mesh = mesh_chunk_with_surface(&chunk, Some(&surface), |x, y, _| {
            let plane = if x < 16 { 10 } else { 5 };
            if y < plane {
                Material::Stone
            } else {
                Material::Air
            }
        });
        assert_eq!(mesh.opaque_surface_quads as usize, mesh.opaque.len());
        assert!(
            mesh.opaque
                .iter()
                .any(|quad| quad.face == FACE_POS_X && quad.origin[0] == 15),
            "the heightfield owns the exact vertical cliff riser"
        );

        let water_surface = flat_surface(coord, 0, Some(5));
        let mut water = Chunk::empty(coord);
        for y in 0..5 {
            for z in 0..CHUNK_EDGE {
                for x in 0..CHUNK_EDGE {
                    water.set(x, y, z, Material::Water);
                }
            }
        }
        let water_mesh = mesh_chunk_with_surface(&water, Some(&water_surface), |_, y, _| {
            if (0..5).contains(&y) {
                Material::Water
            } else {
                Material::Air
            }
        });
        assert_eq!(water_mesh.translucent_surface_quads, 1);
        assert_eq!(
            water_mesh.translucent[0].face, FACE_POS_Y,
            "the heightfield water contract owns only voxel-cell top faces"
        );
    }

    #[test]
    fn boundary_chunk_halos_do_not_wrap_to_the_opposite_world_edge() {
        let minimum = VoxelCoord::new(i32::MIN, 0, 0).chunk();
        let maximum = VoxelCoord::new(i32::MAX, 0, 0).chunk();
        let minimum_mesh = mesh_chunk(&Chunk::filled(minimum, Material::Stone), |x, _, _| {
            assert_ne!(x, i32::MAX);
            Material::Air
        });
        let maximum_mesh = mesh_chunk(&Chunk::filled(maximum, Material::Stone), |x, _, _| {
            assert_ne!(x, i32::MIN);
            Material::Air
        });
        assert_eq!(minimum_mesh.opaque.len(), 6);
        assert_eq!(maximum_mesh.opaque.len(), 6);
    }

    #[test]
    fn opaque_bank_renders_against_water_without_a_duplicate_water_face() {
        let mut chunk = Chunk::empty(ChunkCoord::new(0, 0, 0));
        chunk.set(1, 1, 1, Material::Stone);
        chunk.set(2, 1, 1, Material::Water);
        let mesh = mesh_chunk(&chunk, |_, _, _| Material::Air);
        assert!(mesh.opaque.iter().any(|quad| {
            quad.origin == [1, 1, 1]
                && quad.face == FACE_POS_X
                && quad.material == Material::Stone.id()
        }));
        assert!(!mesh.translucent.iter().any(|quad| {
            quad.origin == [2, 1, 1]
                && quad.face == FACE_NEG_X
                && quad.material == Material::Water.id()
        }));
        assert!(mesh.translucent.iter().any(|quad| {
            quad.origin == [2, 1, 1]
                && quad.face == FACE_POS_Y
                && quad.material == Material::Water.id()
        }));
    }

    #[test]
    fn two_corner_neighbors_fully_occlude_shared_vertex() {
        let mut chunk = Chunk::empty(ChunkCoord::new(0, 0, 0));
        chunk.set(1, 1, 1, Material::Stone);
        chunk.set(0, 2, 1, Material::Stone);
        chunk.set(1, 2, 0, Material::Stone);
        let mesh = mesh_chunk(&chunk, |_, _, _| Material::Air);
        let target = mesh.opaque.iter().find(|quad| {
            quad.face == FACE_POS_Y
                && quad.origin == [1, 1, 1]
                && quad.material == Material::Stone.id()
        });
        assert!(target.is_some());
        if let Some(target) = target {
            assert_eq!(target.ao & 0b11, 0);
        }
    }
}
