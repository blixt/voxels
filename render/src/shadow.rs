//! Pure cascaded directional-shadow projection math.
//!
//! The light direction points from the light toward the scene. Each cascade encloses one camera
//! frustum slice in a square, texel-snapped orthographic projection. Only the receiver slice expands
//! toward the light, keeping otherwise distant casters from destroying depth precision.

use glam::{Mat4, Vec3, Vec4};
use voxels_core::CameraState;

pub const CASCADE_COUNT: usize = 3;

/// Camera and shadow-map parameters used to build the three directional-shadow cascades.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DirectionalShadowConfig {
    pub vertical_fov_radians: f32,
    pub near_plane: f32,
    pub far_plane: f32,
    /// Blend between uniform (`0`) and logarithmic (`1`) cascade splits.
    pub split_lambda: f32,
    pub shadow_map_resolution: u32,
    /// Distance added on the light-facing side of every receiver slice for off-slice casters.
    /// The builder clamps this to `far_plane`, making the expansion finite even for bad input.
    pub caster_depth_expansion: f32,
}

impl Default for DirectionalShadowConfig {
    fn default() -> Self {
        Self {
            vertical_fov_radians: 68.0_f32.to_radians(),
            near_plane: 0.05,
            far_plane: 220.0,
            split_lambda: 0.65,
            shadow_map_resolution: 1_024,
            caster_depth_expansion: 64.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShadowBuildError {
    InvalidAspect,
    InvalidFieldOfView,
    InvalidDepthRange,
    InvalidLightDirection,
    InvalidResolution,
}

#[derive(Clone, Copy, Debug)]
pub struct ShadowCascade {
    /// Maps world positions to WebGPU clip coordinates: XY in `[-1, 1]`, depth in `[0, 1]`.
    pub clip_from_world: Mat4,
    pub split_near: f32,
    pub split_far: f32,
    /// World-space width and height represented by one shadow-map texel.
    pub texel_world_size: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct DirectionalShadowCascades {
    pub cascades: [ShadowCascade; CASCADE_COUNT],
    /// Far view-depth boundary of each cascade, ready for shader cascade selection.
    pub split_depths: [f32; CASCADE_COUNT],
}

/// Build three stable directional-shadow cascades for `camera` and a positive viewport `aspect`.
pub fn build_directional_shadow_cascades(
    camera: &CameraState,
    aspect: f32,
    light_direction: Vec3,
    config: DirectionalShadowConfig,
) -> Result<DirectionalShadowCascades, ShadowBuildError> {
    validate(aspect, light_direction, config)?;

    let lambda = config.split_lambda.clamp(0.0, 1.0);
    let split_depths = practical_splits(config.near_plane, config.far_plane, lambda);
    let light_forward = light_direction.normalize();
    let reference_up = if light_forward.dot(Vec3::Y).abs() > 0.98 {
        Vec3::Z
    } else {
        Vec3::Y
    };
    let light_right = light_forward.cross(reference_up).normalize();
    let light_up = light_right.cross(light_forward).normalize();
    let camera_forward = camera.forward();
    let camera_right = camera_forward.cross(Vec3::Y).normalize();
    let camera_up = camera_right.cross(camera_forward).normalize();
    let tan_half_fov = (config.vertical_fov_radians * 0.5).tan();
    let expansion = config.caster_depth_expansion.clamp(0.0, config.far_plane);

    let cascades = std::array::from_fn(|index| {
        let split_near = if index == 0 {
            config.near_plane
        } else {
            split_depths[index - 1]
        };
        let split_far = split_depths[index];
        let corners = frustum_slice_corners(
            camera.position,
            camera_forward,
            camera_right,
            camera_up,
            aspect,
            tan_half_fov,
            split_near,
            split_far,
        );

        // A rotation-invariant sphere prevents tiny camera rotations from changing the projection
        // scale. Quantizing it also avoids last-bit extent churn from trigonometric evaluation.
        let slice_mid = (split_near + split_far) * 0.5;
        let far_half_height = tan_half_fov * split_far;
        let far_half_width = far_half_height * aspect;
        let near_half_height = tan_half_fov * split_near;
        let near_half_width = near_half_height * aspect;
        let near_radius =
            Vec3::new(near_half_width, near_half_height, split_near - slice_mid).length();
        let far_radius = Vec3::new(far_half_width, far_half_height, split_far - slice_mid).length();
        let quantized_radius = ((near_radius.max(far_radius) * 16.0).ceil() / 16.0).max(1.0 / 16.0);
        // Snapping can move the projection center by half a texel. Reserve enough border for
        // that displacement so every receiver corner remains enclosed after the snap.
        let resolution = config.shadow_map_resolution as f32;
        let radius = quantized_radius / (1.0 - resolution.recip());
        let texel_world_size = radius * 2.0 / config.shadow_map_resolution as f32;
        let slice_center = camera.position + camera_forward * slice_mid;
        let center_x = snap(slice_center.dot(light_right), texel_world_size);
        let center_y = snap(slice_center.dot(light_up), texel_world_size);

        let mut minimum_depth = f32::INFINITY;
        let mut maximum_depth = f32::NEG_INFINITY;
        for corner in corners {
            let depth = corner.dot(light_forward);
            minimum_depth = minimum_depth.min(depth);
            maximum_depth = maximum_depth.max(depth);
        }
        minimum_depth -= expansion;
        let depth_span = (maximum_depth - minimum_depth).max(0.001);

        ShadowCascade {
            clip_from_world: orthographic_light_matrix(
                light_right,
                light_up,
                light_forward,
                center_x,
                center_y,
                radius,
                minimum_depth,
                depth_span,
            ),
            split_near,
            split_far,
            texel_world_size,
        }
    });

    Ok(DirectionalShadowCascades {
        cascades,
        split_depths,
    })
}

/// Conservative clip-volume test for a world-space AABB against one shadow cascade.
pub fn aabb_visible_in_cascade(cascade: &ShadowCascade, minimum: Vec3, maximum: Vec3) -> bool {
    if !minimum.is_finite() || !maximum.is_finite() || minimum.cmpgt(maximum).any() {
        return false;
    }
    let mut clips = [Vec4::ZERO; 8];
    for (index, clip) in clips.iter_mut().enumerate() {
        let corner = Vec3::new(
            if index & 1 == 0 { minimum.x } else { maximum.x },
            if index & 2 == 0 { minimum.y } else { maximum.y },
            if index & 4 == 0 { minimum.z } else { maximum.z },
        );
        *clip = cascade.clip_from_world * corner.extend(1.0);
    }
    !clips.iter().all(|point| point.x < -point.w)
        && !clips.iter().all(|point| point.x > point.w)
        && !clips.iter().all(|point| point.y < -point.w)
        && !clips.iter().all(|point| point.y > point.w)
        && !clips.iter().all(|point| point.z < 0.0)
        && !clips.iter().all(|point| point.z > point.w)
}

fn validate(
    aspect: f32,
    light_direction: Vec3,
    config: DirectionalShadowConfig,
) -> Result<(), ShadowBuildError> {
    if !aspect.is_finite() || aspect <= 0.0 {
        return Err(ShadowBuildError::InvalidAspect);
    }
    if !config.vertical_fov_radians.is_finite()
        || !(0.001..std::f32::consts::PI - 0.001).contains(&config.vertical_fov_radians)
    {
        return Err(ShadowBuildError::InvalidFieldOfView);
    }
    if !config.near_plane.is_finite()
        || !config.far_plane.is_finite()
        || config.near_plane <= 0.0
        || config.far_plane <= config.near_plane
        || !config.split_lambda.is_finite()
        || !config.caster_depth_expansion.is_finite()
    {
        return Err(ShadowBuildError::InvalidDepthRange);
    }
    if !light_direction.is_finite() || light_direction.length_squared() <= f32::EPSILON {
        return Err(ShadowBuildError::InvalidLightDirection);
    }
    if config.shadow_map_resolution < 2 {
        return Err(ShadowBuildError::InvalidResolution);
    }
    Ok(())
}

fn practical_splits(near: f32, far: f32, lambda: f32) -> [f32; CASCADE_COUNT] {
    std::array::from_fn(|index| {
        let fraction = (index + 1) as f32 / CASCADE_COUNT as f32;
        let logarithmic = near * (far / near).powf(fraction);
        let uniform = near + (far - near) * fraction;
        if index + 1 == CASCADE_COUNT {
            far
        } else {
            uniform + (logarithmic - uniform) * lambda
        }
    })
}

#[allow(
    clippy::too_many_arguments,
    reason = "the explicit frustum basis and slice parameters keep this helper independently testable"
)]
fn frustum_slice_corners(
    position: Vec3,
    forward: Vec3,
    right: Vec3,
    up: Vec3,
    aspect: f32,
    tan_half_fov: f32,
    near: f32,
    far: f32,
) -> [Vec3; 8] {
    let mut corners = [Vec3::ZERO; 8];
    for (plane, distance) in [near, far].into_iter().enumerate() {
        let center = position + forward * distance;
        let half_height = tan_half_fov * distance;
        let half_width = half_height * aspect;
        for (local, (x, y)) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)]
            .into_iter()
            .enumerate()
        {
            corners[plane * 4 + local] = center + right * (x * half_width) + up * (y * half_height);
        }
    }
    corners
}

fn snap(value: f32, step: f32) -> f32 {
    (value / step).round() * step
}

#[allow(
    clippy::too_many_arguments,
    reason = "keeping the orthographic basis and bounds explicit makes matrix conventions clear"
)]
fn orthographic_light_matrix(
    right: Vec3,
    up: Vec3,
    forward: Vec3,
    center_x: f32,
    center_y: f32,
    radius: f32,
    minimum_depth: f32,
    depth_span: f32,
) -> Mat4 {
    Mat4::from_cols(
        Vec4::new(right.x / radius, up.x / radius, forward.x / depth_span, 0.0),
        Vec4::new(right.y / radius, up.y / radius, forward.y / depth_span, 0.0),
        Vec4::new(right.z / radius, up.z / radius, forward.z / depth_span, 0.0),
        Vec4::new(
            -center_x / radius,
            -center_y / radius,
            -minimum_depth / depth_span,
            1.0,
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const ASPECT: f32 = 16.0 / 9.0;
    const LIGHT: Vec3 = Vec3::new(0.0, -1.0, -1.0);

    fn build(camera: &CameraState) -> Result<DirectionalShadowCascades, ShadowBuildError> {
        build_directional_shadow_cascades(camera, ASPECT, LIGHT, DirectionalShadowConfig::default())
    }

    fn assert_matrix_close(left: Mat4, right: Mat4, epsilon: f32) {
        for (left, right) in left.to_cols_array().into_iter().zip(right.to_cols_array()) {
            assert!((left - right).abs() <= epsilon, "{left} != {right}");
        }
    }

    #[test]
    fn practical_splits_are_ordered_and_end_at_far_plane() -> Result<(), ShadowBuildError> {
        let config = DirectionalShadowConfig::default();
        let built = build(&CameraState::default())?;
        assert!(built.split_depths[0] > config.near_plane);
        assert!(built.split_depths[0] < built.split_depths[1]);
        assert!(built.split_depths[1] < built.split_depths[2]);
        assert!((built.split_depths[2] - config.far_plane).abs() <= f32::EPSILON);
        Ok(())
    }

    #[test]
    fn every_frustum_slice_corner_is_inside_its_cascade() -> Result<(), ShadowBuildError> {
        let camera = CameraState::from_persisted(Vec3::new(13.0, 8.0, -21.0), 0.73, -0.31);
        let config = DirectionalShadowConfig::default();
        let built = build_directional_shadow_cascades(&camera, ASPECT, LIGHT, config)?;
        let forward = camera.forward();
        let right = forward.cross(Vec3::Y).normalize();
        let up = right.cross(forward).normalize();
        let tan_half_fov = (config.vertical_fov_radians * 0.5).tan();
        for (index, cascade) in built.cascades.iter().enumerate() {
            let corners = frustum_slice_corners(
                camera.position,
                forward,
                right,
                up,
                ASPECT,
                tan_half_fov,
                cascade.split_near,
                cascade.split_far,
            );
            for corner in corners {
                let clip = cascade.clip_from_world * corner.extend(1.0);
                assert!(
                    clip.x >= -1.0001 && clip.x <= 1.0001,
                    "cascade {index}: {clip:?}"
                );
                assert!(
                    clip.y >= -1.0001 && clip.y <= 1.0001,
                    "cascade {index}: {clip:?}"
                );
                assert!(
                    clip.z >= -0.0001 && clip.z <= 1.0001,
                    "cascade {index}: {clip:?}"
                );
            }
        }
        Ok(())
    }

    #[test]
    fn sub_texel_camera_translation_keeps_projection_stable() -> Result<(), ShadowBuildError> {
        let camera = CameraState::from_persisted(Vec3::ZERO, 0.0, 0.0);
        let first = build(&camera)?;
        let mut shifted = camera;
        shifted.position.x += first.cascades[0].texel_world_size * 0.25;
        let second = build(&shifted)?;
        for index in 0..CASCADE_COUNT {
            assert_matrix_close(
                first.cascades[index].clip_from_world,
                second.cascades[index].clip_from_world,
                1.0e-6,
            );
        }
        Ok(())
    }

    #[test]
    fn movement_beyond_one_texel_changes_projection() -> Result<(), ShadowBuildError> {
        let camera = CameraState::from_persisted(Vec3::ZERO, 0.0, 0.0);
        let first = build(&camera)?;
        let mut shifted = camera;
        shifted.position.x += first.cascades[0].texel_world_size * 1.25;
        let second = build(&shifted)?;
        assert_ne!(
            first.cascades[0].clip_from_world,
            second.cascades[0].clip_from_world
        );
        Ok(())
    }

    #[test]
    fn extreme_aspects_stay_finite() -> Result<(), ShadowBuildError> {
        let camera = CameraState::from_persisted(Vec3::new(2.0, 4.0, -8.0), -1.2, 1.45);
        for aspect in [0.05, 12.0] {
            let built = build_directional_shadow_cascades(
                &camera,
                aspect,
                Vec3::new(0.001, -1.0, 0.001),
                DirectionalShadowConfig::default(),
            )?;
            assert!(built.cascades.iter().all(|cascade| {
                cascade.clip_from_world.is_finite()
                    && cascade.texel_world_size.is_finite()
                    && cascade.texel_world_size > 0.0
            }));
        }
        Ok(())
    }

    #[test]
    fn cascade_aabb_test_rejects_separated_bounds() -> Result<(), ShadowBuildError> {
        let built = build(&CameraState::default())?;
        let cascade = &built.cascades[0];
        assert!(aabb_visible_in_cascade(
            cascade,
            Vec3::new(-0.1, 2.0, 4.0),
            Vec3::new(0.1, 2.2, 4.2),
        ));
        assert!(!aabb_visible_in_cascade(
            cascade,
            Vec3::splat(10_000.0),
            Vec3::splat(10_001.0),
        ));
        Ok(())
    }
}
