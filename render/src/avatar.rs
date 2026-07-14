//! One-draw articulated voxel avatars.

use bytemuck::{Pod, Zeroable};
use glam::{Quat, Vec3};
use voxels_core::{PLAYER_EYE_HEIGHT_METRES, RemoteAvatarPose};

const PARTS_PER_AVATAR: usize = 13;
const MAX_AVATARS: usize = 64;
const MAX_PARTS: usize = PARTS_PER_AVATAR * MAX_AVATARS;

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct GpuAvatarPart {
    center_half_x: [f32; 4],
    rotation: [f32; 4],
    half_yz: [f32; 4],
    color: [f32; 4],
}

const _: () = assert!(size_of::<GpuAvatarPart>() == 64);

pub(crate) struct AvatarGpu {
    instance_buffer: wgpu::Buffer,
    scene_pipeline: wgpu::RenderPipeline,
    spatial_ao_pipeline: wgpu::RenderPipeline,
    depth_pipeline: wgpu::RenderPipeline,
    shadow_pipeline: wgpu::RenderPipeline,
    instances: Vec<GpuAvatarPart>,
    instance_count: u32,
    avatar_count: u32,
}

impl AvatarGpu {
    pub(crate) fn new(
        device: &wgpu::Device,
        frame_layout: &wgpu::BindGroupLayout,
        shadow_layout: &wgpu::BindGroupLayout,
        scene_format: wgpu::TextureFormat,
        depth_format: wgpu::TextureFormat,
    ) -> Self {
        let instance_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("articulated avatar instances"),
            size: (MAX_PARTS * size_of::<GpuAvatarPart>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let scene_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("avatar scene pipeline layout"),
            bind_group_layouts: &[Some(frame_layout)],
            immediate_size: 0,
        });
        let shadow_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("avatar shadow pipeline layout"),
                bind_group_layouts: &[Some(shadow_layout)],
                immediate_size: 0,
            });
        let shader = device.create_shader_module(wgpu::include_wgsl!("shaders/avatar.wgsl"));
        let shadow_shader =
            device.create_shader_module(wgpu::include_wgsl!("shaders/avatar_shadow.wgsl"));
        let scene_pipeline = avatar_scene_pipeline(
            device,
            "articulated avatar scene pipeline",
            &scene_layout,
            &shader,
            scene_format,
            depth_format,
            true,
        );
        let spatial_ao_pipeline = avatar_scene_pipeline(
            device,
            "articulated avatar spatial AO pipeline",
            &scene_layout,
            &shader,
            scene_format,
            depth_format,
            false,
        );
        let depth_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("articulated avatar depth pipeline"),
            layout: Some(&scene_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[Some(instance_layout())],
                compilation_options: Default::default(),
            },
            fragment: None,
            primitive: cube_primitive(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: depth_format,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let shadow_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("articulated avatar shadow pipeline"),
            layout: Some(&shadow_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shadow_shader,
                entry_point: Some("vs_main"),
                buffers: &[Some(instance_layout())],
                compilation_options: Default::default(),
            },
            fragment: None,
            primitive: cube_primitive(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: depth_format,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState {
                    constant: 2,
                    slope_scale: 2.0,
                    clamp: 0.0,
                },
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        Self {
            instance_buffer,
            scene_pipeline,
            spatial_ao_pipeline,
            depth_pipeline,
            shadow_pipeline,
            instances: Vec::with_capacity(MAX_PARTS),
            instance_count: 0,
            avatar_count: 0,
        }
    }

    pub(crate) fn prepare(
        &mut self,
        queue: &wgpu::Queue,
        avatars: &[RemoteAvatarPose],
        animation_time_seconds: f32,
    ) {
        self.instances.clear();
        for avatar in avatars.iter().take(MAX_AVATARS) {
            if avatar.eye_position_metres.is_finite() {
                append_avatar_parts(&mut self.instances, avatar, animation_time_seconds);
            }
        }
        self.avatar_count = u32::try_from(self.instances.len() / PARTS_PER_AVATAR).unwrap_or(0);
        self.instance_count = u32::try_from(self.instances.len()).unwrap_or(0);
        if !self.instances.is_empty() {
            queue.write_buffer(
                &self.instance_buffer,
                0,
                bytemuck::cast_slice(&self.instances),
            );
        }
    }

    pub(crate) const fn avatar_count(&self) -> u32 {
        self.avatar_count
    }

    pub(crate) const fn instance_count(&self) -> u32 {
        self.instance_count
    }

    pub(crate) const fn buffer_bytes(&self) -> u64 {
        (MAX_PARTS * size_of::<GpuAvatarPart>()) as u64
    }

    pub(crate) fn draw_scene<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>, spatial_ao: bool) {
        if self.instance_count == 0 {
            return;
        }
        pass.set_pipeline(if spatial_ao {
            &self.spatial_ao_pipeline
        } else {
            &self.scene_pipeline
        });
        pass.set_vertex_buffer(0, self.instance_buffer.slice(..));
        pass.draw(0..36, 0..self.instance_count);
    }

    pub(crate) fn draw_depth<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>) {
        if self.instance_count == 0 {
            return;
        }
        pass.set_pipeline(&self.depth_pipeline);
        pass.set_vertex_buffer(0, self.instance_buffer.slice(..));
        pass.draw(0..36, 0..self.instance_count);
    }

    pub(crate) fn draw_shadow<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>) {
        if self.instance_count == 0 {
            return;
        }
        pass.set_pipeline(&self.shadow_pipeline);
        pass.set_vertex_buffer(0, self.instance_buffer.slice(..));
        pass.draw(0..36, 0..self.instance_count);
    }
}

fn avatar_scene_pipeline(
    device: &wgpu::Device,
    label: &str,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    scene_format: wgpu::TextureFormat,
    depth_format: wgpu::TextureFormat,
    write_depth: bool,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_main"),
            buffers: &[Some(instance_layout())],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("fs_main"),
            targets: &[Some(wgpu::ColorTargetState {
                format: scene_format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: Default::default(),
        }),
        primitive: cube_primitive(),
        depth_stencil: Some(wgpu::DepthStencilState {
            format: depth_format,
            depth_write_enabled: Some(write_depth),
            depth_compare: Some(if write_depth {
                wgpu::CompareFunction::Less
            } else {
                wgpu::CompareFunction::LessEqual
            }),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

fn cube_primitive() -> wgpu::PrimitiveState {
    wgpu::PrimitiveState {
        cull_mode: Some(wgpu::Face::Back),
        ..Default::default()
    }
}

fn instance_layout() -> wgpu::VertexBufferLayout<'static> {
    const ATTRIBUTES: [wgpu::VertexAttribute; 4] = wgpu::vertex_attr_array![
        0 => Float32x4,
        1 => Float32x4,
        2 => Float32x4,
        3 => Float32x4
    ];
    wgpu::VertexBufferLayout {
        array_stride: size_of::<GpuAvatarPart>() as wgpu::BufferAddress,
        step_mode: wgpu::VertexStepMode::Instance,
        attributes: &ATTRIBUTES,
    }
}

fn append_avatar_parts(instances: &mut Vec<GpuAvatarPart>, avatar: &RemoteAvatarPose, time: f32) {
    let color = saturated_player_color(avatar.color_index);
    let body_rotation = Quat::from_rotation_y(avatar.body_yaw_radians);
    let speed_blend = ((avatar.locomotion_speed_metres_per_second - 0.08) / 2.35).clamp(0.0, 1.0);
    let phase = avatar.gait_phase_radians;
    let idle_offset = f32::from(avatar.color_index) * 0.73;
    let breathing = (time * 1.65 + idle_offset).sin() * 0.006 * (1.0 - speed_blend);
    let bob = phase.mul_add(2.0, 0.0).cos() * 0.018 * speed_blend + breathing;
    let sway = phase.sin() * 0.014 * speed_blend;
    let ground = avatar.eye_position_metres - Vec3::Y * PLAYER_EYE_HEIGHT_METRES;
    let body_origin = ground + body_rotation * Vec3::new(sway, bob, 0.0);
    let lean =
        (0.025 + avatar.locomotion_speed_metres_per_second * 0.012).clamp(0.0, 0.105) * speed_blend;
    let torso_rotation = body_rotation * Quat::from_rotation_x(lean);
    let hip_center = body_origin + body_rotation * Vec3::new(0.0, 0.91, 0.0);
    let torso_center = hip_center + torso_rotation * Vec3::new(0.0, 0.245, 0.0);
    instances.push(part(
        torso_center,
        torso_rotation,
        Vec3::new(0.155, 0.245 + breathing.abs() * 0.25, 0.09),
        color,
    ));

    let neck = hip_center + torso_rotation * Vec3::new(0.0, 0.51, 0.0);
    let head_rotation = body_rotation
        * Quat::from_rotation_y(avatar.head_yaw_radians)
        * Quat::from_rotation_x(avatar.look_pitch_radians);
    let head_center = neck + head_rotation * Vec3::new(0.0, 0.145, 0.0);
    instances.push(part(
        head_center,
        head_rotation,
        Vec3::new(0.13, 0.14, 0.115),
        color,
    ));
    // A same-color face nub makes look direction readable from silhouette while preserving the
    // avatar's deliberately simple single-color graphic language.
    instances.push(part(
        head_center + head_rotation * Vec3::new(0.0, -0.015, -0.14),
        head_rotation,
        Vec3::new(0.06, 0.035, 0.035),
        color,
    ));

    for side in [-1.0_f32, 1.0] {
        let leg_phase = if side < 0.0 {
            phase
        } else {
            phase + std::f32::consts::PI
        };
        let swing = leg_phase.sin() * 0.62 * speed_blend;
        let knee_bend = (0.08 + (-leg_phase.sin()).max(0.0) * 0.82) * speed_blend;
        let hip = hip_center + body_rotation * Vec3::new(side * 0.085, 0.0, 0.0);
        let thigh_direction = body_rotation * (Quat::from_rotation_x(swing) * Vec3::NEG_Y);
        let knee = hip + thigh_direction * 0.405;
        let shin_direction =
            body_rotation * (Quat::from_rotation_x(swing + knee_bend) * Vec3::NEG_Y);
        let ankle = knee + shin_direction * 0.405;
        instances.push(segment(hip, knee, 0.095, 0.095, color));
        instances.push(segment(knee, ankle, 0.09, 0.09, color));
        let foot_center = ankle + body_rotation * Vec3::new(0.0, -0.005, -0.065);
        instances.push(part(
            foot_center,
            body_rotation,
            Vec3::new(0.052, 0.052, 0.115),
            color,
        ));

        let arm_swing = -leg_phase.sin() * 0.52 * speed_blend;
        let shoulder = hip_center + torso_rotation * Vec3::new(side * 0.205, 0.46, -0.005);
        let upper_direction = body_rotation * (Quat::from_rotation_x(arm_swing) * Vec3::NEG_Y);
        let elbow = shoulder + upper_direction * 0.33;
        let elbow_bend = (0.08 + leg_phase.sin().max(0.0) * 0.22) * speed_blend;
        let forearm_direction =
            body_rotation * (Quat::from_rotation_x(arm_swing * 0.62 - elbow_bend) * Vec3::NEG_Y);
        let hand = elbow + forearm_direction * 0.325;
        instances.push(segment(shoulder, elbow, 0.085, 0.085, color));
        instances.push(segment(elbow, hand, 0.08, 0.08, color));
    }
    debug_assert_eq!(instances.len() % PARTS_PER_AVATAR, 0);
}

fn segment(start: Vec3, end: Vec3, width: f32, depth: f32, color: Vec3) -> GpuAvatarPart {
    let delta = end - start;
    let length = delta.length().max(0.001);
    let rotation = Quat::from_rotation_arc(Vec3::Y, delta / length);
    part(
        (start + end) * 0.5,
        rotation,
        Vec3::new(width * 0.5, length * 0.5, depth * 0.5),
        color,
    )
}

fn part(center: Vec3, rotation: Quat, half: Vec3, color: Vec3) -> GpuAvatarPart {
    GpuAvatarPart {
        center_half_x: [center.x, center.y, center.z, half.x],
        rotation: rotation.to_array(),
        half_yz: [half.y, half.z, 0.0, 0.0],
        color: [color.x, color.y, color.z, 0.72],
    }
}

fn saturated_player_color(index: u8) -> Vec3 {
    let hue = (f32::from(index) * 0.618_034 + 0.035).fract();
    hsv_to_rgb(hue, 0.78, 0.96)
}

fn hsv_to_rgb(hue: f32, saturation: f32, value: f32) -> Vec3 {
    let scaled = hue * 6.0;
    let sector = scaled.floor() as i32;
    let fraction = scaled - sector as f32;
    let low = value * (1.0 - saturation);
    let falling = value * (1.0 - saturation * fraction);
    let rising = value * (1.0 - saturation * (1.0 - fraction));
    match sector.rem_euclid(6) {
        0 => Vec3::new(value, rising, low),
        1 => Vec3::new(falling, value, low),
        2 => Vec3::new(low, value, rising),
        3 => Vec3::new(low, falling, value),
        4 => Vec3::new(rising, low, value),
        _ => Vec3::new(value, low, falling),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use voxels_core::RemotePlayerId;

    fn avatar() -> RemoteAvatarPose {
        RemoteAvatarPose {
            player_id: RemotePlayerId([1; 16]),
            connection_id: 1,
            color_index: 0,
            eye_position_metres: Vec3::new(2.0, PLAYER_EYE_HEIGHT_METRES, 3.0),
            linear_velocity_metres_per_second: Vec3::new(1.0, 0.0, 0.0),
            look_yaw_radians: 0.2,
            look_pitch_radians: -0.1,
            body_yaw_radians: 0.0,
            head_yaw_radians: 0.2,
            gait_phase_radians: 1.2,
            locomotion_speed_metres_per_second: 2.0,
            flags: 0,
            extrapolated: false,
        }
    }

    #[test]
    fn articulated_avatar_has_exactly_thirteen_finite_cuboids() {
        let mut instances = Vec::new();
        append_avatar_parts(&mut instances, &avatar(), 1.0);
        assert_eq!(instances.len(), PARTS_PER_AVATAR);
        assert!(instances.iter().all(|instance| {
            instance.center_half_x.into_iter().all(f32::is_finite)
                && instance.rotation.into_iter().all(f32::is_finite)
                && instance.half_yz.into_iter().all(f32::is_finite)
        }));
    }

    #[test]
    fn palette_is_bright_saturated_and_unique_for_the_server_limit() {
        let colors = (0..MAX_AVATARS as u8)
            .map(saturated_player_color)
            .collect::<Vec<_>>();
        for color in &colors {
            assert!(color.max_element() >= 0.95);
            assert!(color.max_element() - color.min_element() >= 0.70);
        }
        for (index, color) in colors.iter().enumerate() {
            assert!(colors.iter().skip(index + 1).all(|other| *other != *color));
        }
    }

    #[test]
    fn head_turn_changes_head_without_rotating_the_torso() {
        let mut straight = avatar();
        straight.head_yaw_radians = 0.0;
        let mut looking = straight;
        looking.head_yaw_radians = 0.8;
        let mut first = Vec::new();
        let mut second = Vec::new();
        append_avatar_parts(&mut first, &straight, 0.0);
        append_avatar_parts(&mut second, &looking, 0.0);
        assert_eq!(first[0].rotation, second[0].rotation);
        assert_ne!(first[1].rotation, second[1].rotation);
        assert_ne!(first[2].rotation, second[2].rotation);
    }
}
