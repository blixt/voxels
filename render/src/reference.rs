//! Exact 10 cm ray-cast reference for bounded camera frusta.
//!
//! This is deliberately independent of the production mesh hierarchy. It answers the canonical
//! question directly for every pixel center: which exact voxel face is first along this ray? That
//! makes it an oracle for holes, wrong material boundaries, depth error, and ownership diagnostics
//! without relying on visual interpretation or reproducing the implementation under test.

use glam::Vec3;
use voxels_world::{CHUNK_EDGE, ChunkCoord, Material, VOXEL_SIZE_METRES};

const OWNER_HASH_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const OWNER_HASH_PRIME: u64 = 0x100_0000_01b3;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReferenceVoxel {
    pub material: Material,
    pub revision: u64,
}

impl ReferenceVoxel {
    pub const AIR: Self = Self {
        material: Material::Air,
        revision: 0,
    };
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ReferenceCamera {
    pub eye_metres: Vec3,
    pub forward: Vec3,
    pub vertical_fov_radians: f32,
    pub near_plane_metres: f32,
    pub far_plane_metres: f32,
    pub pixel_width: u32,
    pub pixel_height: u32,
}

impl ReferenceCamera {
    pub fn is_valid(self) -> bool {
        self.eye_metres.is_finite()
            && self.forward.is_finite()
            && self.forward.length_squared() > f32::EPSILON
            && self.vertical_fov_radians.is_finite()
            && (0.001..std::f32::consts::PI - 0.001).contains(&self.vertical_fov_radians)
            && self.near_plane_metres.is_finite()
            && self.near_plane_metres > 0.0
            && self.far_plane_metres.is_finite()
            && self.far_plane_metres > self.near_plane_metres
            && self.pixel_width > 0
            && self.pixel_height > 0
    }
}

/// Half-open canonical voxel bounds. Sampling outside the bounds is always air.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReferenceBounds {
    pub min: [i32; 3],
    pub max: [i32; 3],
}

impl ReferenceBounds {
    pub fn is_valid(self) -> bool {
        (0..3).all(|axis| self.min[axis] < self.max[axis])
    }

    pub fn contains(self, voxel: [i32; 3]) -> bool {
        (0..3).all(|axis| voxel[axis] >= self.min[axis] && voxel[axis] < self.max[axis])
    }
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReferenceRepresentation {
    ExactCanonical = 1,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ReferenceSample {
    /// Stable hash of the canonical chunk page containing the voxel.
    pub owner_page_id: u64,
    pub representation: ReferenceRepresentation,
    pub hierarchy_depth: u8,
    /// Stable hash of `(voxel x/y/z, entered face)`.
    pub primitive_face_key: u64,
    pub material_id: u16,
    pub revision: u64,
    pub reverse_z_depth: f32,
    pub world_position_metres: [f32; 3],
    pub voxel: [i32; 3],
    pub face_normal: [i8; 3],
}

#[derive(Clone, Debug, PartialEq)]
pub struct ExactReferenceFrame {
    pub width: u32,
    pub height: u32,
    pub samples: Vec<Option<ReferenceSample>>,
}

impl ExactReferenceFrame {
    pub fn sample(&self, x: u32, y: u32) -> Option<ReferenceSample> {
        if x >= self.width || y >= self.height {
            return None;
        }
        self.samples[(x + y * self.width) as usize]
    }

    pub fn surface_pixel_count(&self) -> usize {
        self.samples
            .iter()
            .filter(|sample| sample.is_some())
            .count()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ReferenceComparison {
    pub expected_surface_pixels: u64,
    pub ownerless_visible_samples: u64,
    pub material_mismatches: u64,
    pub maximum_depth_error: f32,
}

/// Compares a renderer diagnostic attachment to the exact reference.
///
/// Candidate samples need not use exact-page IDs: representation experiments can still be judged
/// objectively by coverage, material, and depth while separately testing their own unique-owner
/// contract.
pub fn compare_reference(
    reference: &ExactReferenceFrame,
    candidate: &[Option<ReferenceSample>],
) -> Option<ReferenceComparison> {
    if candidate.len() != reference.samples.len() {
        return None;
    }
    let mut comparison = ReferenceComparison::default();
    for (expected, actual) in reference.samples.iter().zip(candidate) {
        let Some(expected) = expected else {
            continue;
        };
        comparison.expected_surface_pixels += 1;
        let Some(actual) = actual else {
            comparison.ownerless_visible_samples += 1;
            continue;
        };
        comparison.material_mismatches += u64::from(actual.material_id != expected.material_id);
        comparison.maximum_depth_error = comparison
            .maximum_depth_error
            .max((actual.reverse_z_depth - expected.reverse_z_depth).abs());
    }
    Some(comparison)
}

pub fn render_exact_reference(
    camera: ReferenceCamera,
    bounds: ReferenceBounds,
    mut voxel_at: impl FnMut(i32, i32, i32) -> ReferenceVoxel,
) -> Option<ExactReferenceFrame> {
    if !camera.is_valid() || !bounds.is_valid() {
        return None;
    }
    let pixel_count = usize::try_from(camera.pixel_width)
        .ok()?
        .checked_mul(usize::try_from(camera.pixel_height).ok()?)?;
    let forward = camera.forward.normalize();
    let world_up = if forward.y.abs() > 0.999 {
        Vec3::Z
    } else {
        Vec3::Y
    };
    let right = forward.cross(world_up).normalize();
    let up = right.cross(forward).normalize();
    let aspect = camera.pixel_width as f32 / camera.pixel_height as f32;
    let tan_half_fov = (camera.vertical_fov_radians * 0.5).tan();
    let mut samples = Vec::with_capacity(pixel_count);
    for y in 0..camera.pixel_height {
        let ndc_y = 1.0 - (y as f32 + 0.5) * 2.0 / camera.pixel_height as f32;
        for x in 0..camera.pixel_width {
            let ndc_x = (x as f32 + 0.5) * 2.0 / camera.pixel_width as f32 - 1.0;
            let direction =
                (forward + right * ndc_x * aspect * tan_half_fov + up * ndc_y * tan_half_fov)
                    .normalize();
            samples.push(trace_exact_voxels(camera, bounds, direction, &mut voxel_at));
        }
    }
    Some(ExactReferenceFrame {
        width: camera.pixel_width,
        height: camera.pixel_height,
        samples,
    })
}

fn trace_exact_voxels(
    camera: ReferenceCamera,
    bounds: ReferenceBounds,
    direction: Vec3,
    voxel_at: &mut impl FnMut(i32, i32, i32) -> ReferenceVoxel,
) -> Option<ReferenceSample> {
    let start = camera.eye_metres + direction * camera.near_plane_metres;
    let mut voxel = (start / VOXEL_SIZE_METRES).floor().as_ivec3();
    let axis_step = |value: f32| {
        if value > 0.0 {
            1
        } else if value < 0.0 {
            -1
        } else {
            0
        }
    };
    let step = glam::IVec3::new(
        axis_step(direction.x),
        axis_step(direction.y),
        axis_step(direction.z),
    );
    let mut maximum = Vec3::splat(f32::INFINITY);
    let mut delta = Vec3::splat(f32::INFINITY);
    for axis in 0..3 {
        if step[axis] == 0 {
            continue;
        }
        let boundary_voxel = if step[axis] > 0 {
            voxel[axis].checked_add(1)?
        } else {
            voxel[axis]
        };
        let boundary_metres = boundary_voxel as f32 * VOXEL_SIZE_METRES;
        maximum[axis] =
            camera.near_plane_metres + (boundary_metres - start[axis]) / direction[axis];
        delta[axis] = VOXEL_SIZE_METRES / direction[axis].abs();
    }
    let mut entered_face = [0_i8; 3];
    loop {
        let world_voxel = voxel.to_array();
        if bounds.contains(world_voxel) {
            let value = voxel_at(voxel.x, voxel.y, voxel.z);
            if value.material.is_renderable() {
                let distance = entry_distance(camera.near_plane_metres, maximum, delta);
                let view_depth = (distance * direction.dot(camera.forward.normalize()))
                    .max(camera.near_plane_metres);
                let world_position = camera.eye_metres + direction * distance;
                return Some(ReferenceSample {
                    owner_page_id: canonical_page_id(VoxelPage::from_voxel(world_voxel)),
                    representation: ReferenceRepresentation::ExactCanonical,
                    hierarchy_depth: 0,
                    primitive_face_key: primitive_face_key(world_voxel, entered_face),
                    material_id: value.material.id(),
                    revision: value.revision,
                    reverse_z_depth: reverse_z_depth(
                        view_depth,
                        camera.near_plane_metres,
                        camera.far_plane_metres,
                    ),
                    world_position_metres: world_position.to_array(),
                    voxel: world_voxel,
                    face_normal: entered_face,
                });
            }
        }
        let axis = minimum_axis(maximum);
        let distance = maximum[axis];
        if !distance.is_finite() || distance > camera.far_plane_metres {
            return None;
        }
        voxel[axis] = voxel[axis].checked_add(step[axis])?;
        entered_face = [0; 3];
        entered_face[axis] = (-step[axis]) as i8;
        maximum[axis] += delta[axis];
    }
}

fn entry_distance(near: f32, maximum: Vec3, delta: Vec3) -> f32 {
    let previous = Vec3::new(
        maximum.x - delta.x,
        maximum.y - delta.y,
        maximum.z - delta.z,
    );
    previous
        .to_array()
        .into_iter()
        .filter(|value| value.is_finite())
        .fold(near, f32::max)
}

fn minimum_axis(value: Vec3) -> usize {
    if value.x <= value.y && value.x <= value.z {
        0
    } else if value.y <= value.z {
        1
    } else {
        2
    }
}

fn reverse_z_depth(view_depth: f32, near: f32, far: f32) -> f32 {
    (near * (far / view_depth - 1.0) / (far - near)).clamp(0.0, 1.0)
}

#[derive(Clone, Copy)]
struct VoxelPage(ChunkCoord);

impl VoxelPage {
    fn from_voxel(voxel: [i32; 3]) -> Self {
        let edge = CHUNK_EDGE as i32;
        Self(ChunkCoord::new(
            voxel[0].div_euclid(edge),
            voxel[1].div_euclid(edge),
            voxel[2].div_euclid(edge),
        ))
    }
}

fn canonical_page_id(page: VoxelPage) -> u64 {
    stable_hash(&[
        1,
        page.0.x as u32 as u64,
        page.0.y as u32 as u64,
        page.0.z as u32 as u64,
    ])
}

fn primitive_face_key(voxel: [i32; 3], normal: [i8; 3]) -> u64 {
    stable_hash(&[
        voxel[0] as u32 as u64,
        voxel[1] as u32 as u64,
        voxel[2] as u32 as u64,
        normal[0] as u8 as u64,
        normal[1] as u8 as u64,
        normal[2] as u8 as u64,
    ])
}

fn stable_hash(values: &[u64]) -> u64 {
    values.iter().fold(OWNER_HASH_OFFSET, |hash, value| {
        value.to_le_bytes().iter().fold(hash, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(OWNER_HASH_PRIME)
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn camera(eye: Vec3, forward: Vec3, width: u32, height: u32) -> ReferenceCamera {
        ReferenceCamera {
            eye_metres: eye,
            forward,
            vertical_fov_radians: 68.0_f32.to_radians(),
            near_plane_metres: 0.05,
            far_plane_metres: 20.0,
            pixel_width: width,
            pixel_height: height,
        }
    }

    #[test]
    fn exact_wall_has_one_stable_owner_at_every_covered_pixel() {
        let bounds = ReferenceBounds {
            min: [-32, -32, -32],
            max: [32, 32, 32],
        };
        let frame = render_exact_reference(
            camera(Vec3::new(0.0, 0.0, 2.0), -Vec3::Z, 63, 41),
            bounds,
            |_x, _y, z| {
                if z == 0 {
                    ReferenceVoxel {
                        material: Material::Stone,
                        revision: 7,
                    }
                } else {
                    ReferenceVoxel::AIR
                }
            },
        )
        .expect("valid reference");
        assert_eq!(frame.surface_pixel_count(), frame.samples.len());
        assert!(frame.samples.iter().all(|sample| {
            sample.is_some_and(|sample| {
                sample.material_id == Material::Stone.id()
                    && sample.revision == 7
                    && sample.face_normal == [0, 0, 1]
            })
        }));
    }

    #[test]
    fn negative_coordinate_faces_use_euclidean_chunk_pages() {
        let bounds = ReferenceBounds {
            min: [-64, -4, -64],
            max: [4, 4, 4],
        };
        let frame = render_exact_reference(
            camera(Vec3::new(-3.15, 0.05, 1.0), -Vec3::Z, 1, 1),
            bounds,
            |x, y, z| {
                if x == -32 && y == 0 && z == 0 {
                    ReferenceVoxel {
                        material: Material::Grass,
                        revision: 1,
                    }
                } else {
                    ReferenceVoxel::AIR
                }
            },
        )
        .expect("valid reference");
        let sample = frame.sample(0, 0).expect("negative voxel is visible");
        assert_eq!(sample.voxel, [-32, 0, 0]);
        assert_eq!(
            sample.owner_page_id,
            canonical_page_id(VoxelPage(ChunkCoord::new(-1, 0, 0)))
        );
    }

    #[test]
    fn comparison_counts_machine_detectable_holes_and_material_errors() {
        let expected = ReferenceSample {
            owner_page_id: 1,
            representation: ReferenceRepresentation::ExactCanonical,
            hierarchy_depth: 0,
            primitive_face_key: 2,
            material_id: Material::Stone.id(),
            revision: 1,
            reverse_z_depth: 0.5,
            world_position_metres: [0.0; 3],
            voxel: [0; 3],
            face_normal: [0, 0, 1],
        };
        let reference = ExactReferenceFrame {
            width: 2,
            height: 1,
            samples: vec![Some(expected), Some(expected)],
        };
        let wrong = ReferenceSample {
            material_id: Material::Dirt.id(),
            reverse_z_depth: 0.25,
            ..expected
        };
        let comparison =
            compare_reference(&reference, &[None, Some(wrong)]).expect("matching dimensions");
        assert_eq!(comparison.expected_surface_pixels, 2);
        assert_eq!(comparison.ownerless_visible_samples, 1);
        assert_eq!(comparison.material_mismatches, 1);
        assert_eq!(comparison.maximum_depth_error, 0.25);
    }
}
