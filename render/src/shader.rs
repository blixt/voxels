use std::borrow::Cow;

const FRAME_SOURCE: &str = include_str!("shaders/frame.wgsl");

pub(crate) fn frame_shader(
    device: &wgpu::Device,
    label: &'static str,
    source: &'static str,
) -> wgpu::ShaderModule {
    device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(label),
        source: wgpu::ShaderSource::Wgsl(Cow::Owned(format!("{FRAME_SOURCE}\n{source}"))),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_frame_source_matches_the_host_uniform_order() {
        let fields = FRAME_SOURCE
            .lines()
            .filter_map(|line| line.trim().split_once(':').map(|(name, _)| name))
            .collect::<Vec<_>>();
        assert_eq!(
            fields,
            [
                "view_projection",
                "inverse_view_projection",
                "camera_time",
                "viewport_voxel",
                "target_voxel",
                "target_voxel_max",
                "render_options",
                "lod_options",
                "lod_boundary_centres",
                "camera_forward",
                "shadow_splits",
                "shadow_texel_sizes",
                "shadow_view_projection",
                "sun_direction",
                "sun_radiance",
                "sky_horizon",
                "sky_zenith",
                "ground_atmosphere",
                "fog_exposure",
                "medium",
                "interior",
            ]
        );
    }
}
