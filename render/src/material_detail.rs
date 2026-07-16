use voxels_world::Material;
use wgpu::{
    AddressMode, Device, Extent3d, FilterMode, MipmapFilterMode, Origin3d, Queue, Sampler,
    SamplerDescriptor, TexelCopyBufferLayout, TexelCopyTextureInfo, Texture, TextureAspect,
    TextureDescriptor, TextureDimension, TextureFormat, TextureUsages, TextureView,
    TextureViewDescriptor, TextureViewDimension,
};

pub(crate) const MATERIAL_DETAIL_SIZE: u32 = 128;
pub(crate) const MATERIAL_DETAIL_MIP_COUNT: u32 = MATERIAL_DETAIL_SIZE.ilog2() + 1;
pub(crate) const MATERIAL_DETAIL_LAYER_COUNT: u32 = Material::ALL.len() as u32;

#[cfg(test)]
const MATERIAL_TEXELS_PER_VOXEL: i32 = 3;

#[derive(Clone, Copy)]
struct MaterialProfile {
    base_srgb: [f32; 3],
    accent_srgb: [f32; 3],
    roughness: f32,
    roughness_variation: f32,
    normal_strength: f32,
}

#[derive(Clone, Debug)]
struct LayerTexels {
    albedo: Vec<[f32; 3]>,
    normal_roughness: Vec<[f32; 4]>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct MipBytes {
    width: u32,
    albedo: Vec<u8>,
    normal_roughness: Vec<u8>,
}

#[derive(Clone, Debug)]
struct MaterialDetailAtlas {
    mips: Vec<MipBytes>,
}

pub(crate) struct MaterialDetailGpu {
    _albedo_texture: Texture,
    _normal_roughness_texture: Texture,
    pub(crate) albedo_view: TextureView,
    pub(crate) normal_roughness_view: TextureView,
    pub(crate) sampler: Sampler,
    pub(crate) bytes: u64,
}

fn material_detail_sampler_descriptor() -> SamplerDescriptor<'static> {
    SamplerDescriptor {
        label: Some("pixelated material detail sampler"),
        address_mode_u: AddressMode::Repeat,
        address_mode_v: AddressMode::Repeat,
        address_mode_w: AddressMode::ClampToEdge,
        // Preserve the authored texel blocks. Linear magnification was smearing the procedural
        // atlas across 10 cm voxel faces and fighting the deliberately pixelated world style.
        mag_filter: FilterMode::Nearest,
        // Once a texel is smaller than a screen pixel, blend both within and between mip levels.
        // This keeps the close pixel blocks crisp while preventing distant roughness and normals
        // from stepping or shimmering as the camera moves.
        min_filter: FilterMode::Linear,
        mipmap_filter: MipmapFilterMode::Linear,
        anisotropy_clamp: 1,
        ..Default::default()
    }
}

impl MaterialDetailGpu {
    pub(crate) fn new(device: &Device, queue: &Queue) -> Self {
        let atlas = MaterialDetailAtlas::generate();
        let albedo_texture = texture(
            device,
            "material detail albedo",
            TextureFormat::Rgba8UnormSrgb,
        );
        let normal_roughness_texture = texture(
            device,
            "material detail normal roughness",
            TextureFormat::Rgba8Unorm,
        );
        for (mip_level, mip) in atlas.mips.iter().enumerate() {
            let extent = Extent3d {
                width: mip.width,
                height: mip.width,
                depth_or_array_layers: MATERIAL_DETAIL_LAYER_COUNT,
            };
            let layout = TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(mip.width * 4),
                rows_per_image: Some(mip.width),
            };
            queue.write_texture(
                TexelCopyTextureInfo {
                    texture: &albedo_texture,
                    mip_level: mip_level as u32,
                    origin: Origin3d::ZERO,
                    aspect: TextureAspect::All,
                },
                &mip.albedo,
                layout,
                extent,
            );
            queue.write_texture(
                TexelCopyTextureInfo {
                    texture: &normal_roughness_texture,
                    mip_level: mip_level as u32,
                    origin: Origin3d::ZERO,
                    aspect: TextureAspect::All,
                },
                &mip.normal_roughness,
                layout,
                extent,
            );
        }
        let view_descriptor = TextureViewDescriptor {
            label: Some("material detail array view"),
            dimension: Some(TextureViewDimension::D2Array),
            ..Default::default()
        };
        let albedo_view = albedo_texture.create_view(&view_descriptor);
        let normal_roughness_view = normal_roughness_texture.create_view(&view_descriptor);
        let sampler = device.create_sampler(&material_detail_sampler_descriptor());
        Self {
            _albedo_texture: albedo_texture,
            _normal_roughness_texture: normal_roughness_texture,
            albedo_view,
            normal_roughness_view,
            sampler,
            bytes: atlas.byte_len() as u64,
        }
    }
}

impl MaterialDetailAtlas {
    fn generate() -> Self {
        let mut mips = (0..MATERIAL_DETAIL_MIP_COUNT)
            .map(|level| {
                let width = MATERIAL_DETAIL_SIZE >> level;
                let capacity =
                    width as usize * width as usize * MATERIAL_DETAIL_LAYER_COUNT as usize * 4;
                MipBytes {
                    width,
                    albedo: Vec::with_capacity(capacity),
                    normal_roughness: Vec::with_capacity(capacity),
                }
            })
            .collect::<Vec<_>>();
        for material in Material::ALL {
            let mut width = MATERIAL_DETAIL_SIZE;
            let mut layer = generate_layer(material, width);
            for mip in &mut mips {
                encode_layer(&layer, mip);
                if width > 1 {
                    layer = downsample_layer(&layer, width);
                    width /= 2;
                }
            }
        }
        Self { mips }
    }

    fn byte_len(&self) -> usize {
        self.mips
            .iter()
            .map(|mip| mip.albedo.len() + mip.normal_roughness.len())
            .sum()
    }
}

fn texture(device: &Device, label: &'static str, format: TextureFormat) -> Texture {
    device.create_texture(&TextureDescriptor {
        label: Some(label),
        size: Extent3d {
            width: MATERIAL_DETAIL_SIZE,
            height: MATERIAL_DETAIL_SIZE,
            depth_or_array_layers: MATERIAL_DETAIL_LAYER_COUNT,
        },
        mip_level_count: MATERIAL_DETAIL_MIP_COUNT,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format,
        usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
        view_formats: &[],
    })
}

fn generate_layer(material: Material, width: u32) -> LayerTexels {
    let profile = material_profile(material);
    let mut heights = Vec::with_capacity(width as usize * width as usize);
    for y in 0..width {
        for x in 0..width {
            heights.push(material_height(
                material,
                x as f32 / width as f32,
                y as f32 / width as f32,
            ));
        }
    }
    let mut albedo = Vec::with_capacity(width as usize * width as usize);
    let mut normal_roughness = Vec::with_capacity(width as usize * width as usize);
    for y in 0..width {
        for x in 0..width {
            let u = x as f32 / width as f32;
            let v = y as f32 / width as f32;
            let broad = periodic_noise(material, u, v, 4, 0x51f7);
            let detail = periodic_noise(material, u, v, 16, 0x9e37);
            let pattern = material_pattern(material, u, v);
            let blend = (broad * 0.66 + detail * 0.22 + pattern * 0.12).clamp(0.0, 1.0);
            let base = profile.base_srgb.map(srgb_to_linear);
            let accent = profile.accent_srgb.map(srgb_to_linear);
            let tone = 0.88 + (detail - 0.5) * 0.20;
            albedo.push(std::array::from_fn(|channel| {
                (base[channel] + (accent[channel] - base[channel]) * blend) * tone
            }));

            let left = heights[((x + width - 1) % width + y * width) as usize];
            let right = heights[((x + 1) % width + y * width) as usize];
            let down = heights[(x + ((y + width - 1) % width) * width) as usize];
            let up = heights[(x + ((y + 1) % width) * width) as usize];
            let normal = glam::Vec3::new(
                (left - right) * profile.normal_strength,
                (down - up) * profile.normal_strength,
                1.0,
            )
            .normalize_or_zero();
            let roughness = (profile.roughness
                + (periodic_noise(material, u, v, 32, 0xb529) - 0.5) * profile.roughness_variation)
                .clamp(0.08, 1.0);
            normal_roughness.push([normal.x, normal.y, normal.z, roughness]);
        }
    }
    LayerTexels {
        albedo,
        normal_roughness,
    }
}

fn downsample_layer(source: &LayerTexels, width: u32) -> LayerTexels {
    let next_width = width / 2;
    let mut albedo = Vec::with_capacity(next_width as usize * next_width as usize);
    let mut normal_roughness = Vec::with_capacity(next_width as usize * next_width as usize);
    for y in 0..next_width {
        for x in 0..next_width {
            let indices = [
                (x * 2 + (y * 2) * width) as usize,
                (x * 2 + 1 + (y * 2) * width) as usize,
                (x * 2 + (y * 2 + 1) * width) as usize,
                (x * 2 + 1 + (y * 2 + 1) * width) as usize,
            ];
            albedo.push(std::array::from_fn(|channel| {
                indices
                    .iter()
                    .map(|index| source.albedo[*index][channel])
                    .sum::<f32>()
                    * 0.25
            }));
            let normal = indices.iter().fold(glam::Vec3::ZERO, |sum, index| {
                let value = source.normal_roughness[*index];
                sum + glam::Vec3::new(value[0], value[1], value[2])
            }) * 0.25;
            let roughness_squared = indices
                .iter()
                .map(|index| source.normal_roughness[*index][3].powi(2))
                .sum::<f32>()
                * 0.25;
            normal_roughness.push([normal.x, normal.y, normal.z, roughness_squared.sqrt()]);
        }
    }
    LayerTexels {
        albedo,
        normal_roughness,
    }
}

fn encode_layer(layer: &LayerTexels, destination: &mut MipBytes) {
    for color in &layer.albedo {
        destination.albedo.extend(
            color
                .map(|channel| encode_unorm(linear_to_srgb(channel.max(0.0))))
                .into_iter()
                .chain([255]),
        );
    }
    for value in &layer.normal_roughness {
        destination.normal_roughness.extend([
            encode_unorm(value[0] * 0.5 + 0.5),
            encode_unorm(value[1] * 0.5 + 0.5),
            encode_unorm(value[2] * 0.5 + 0.5),
            encode_unorm(value[3]),
        ]);
    }
}

fn material_profile(material: Material) -> MaterialProfile {
    match material {
        Material::Air => profile([0.5, 0.0, 0.5], [0.65, 0.0, 0.65], 1.0, 0.0, 0.0),
        Material::Grass => profile([0.18, 0.42, 0.12], [0.24, 0.45, 0.10], 0.90, 0.06, 1.8),
        Material::Dirt => profile([0.36, 0.20, 0.095], [0.24, 0.12, 0.055], 0.96, 0.05, 2.5),
        Material::Stone => profile([0.34, 0.38, 0.43], [0.22, 0.25, 0.30], 0.82, 0.10, 3.4),
        Material::Sand => profile([0.58, 0.43, 0.24], [0.72, 0.56, 0.31], 0.95, 0.04, 2.0),
        Material::Snow => profile([0.76, 0.86, 0.91], [0.92, 0.96, 1.0], 0.82, 0.10, 1.6),
        Material::Clay => profile([0.56, 0.25, 0.15], [0.68, 0.31, 0.18], 0.90, 0.06, 2.5),
        Material::Basalt => profile([0.12, 0.15, 0.20], [0.20, 0.22, 0.25], 0.84, 0.09, 4.0),
        Material::Wood => profile([0.31, 0.15, 0.055], [0.49, 0.25, 0.085], 0.84, 0.07, 4.0),
        Material::Leaves => profile([0.08, 0.30, 0.10], [0.18, 0.42, 0.12], 0.84, 0.10, 2.8),
        Material::Moss => profile([0.12, 0.32, 0.14], [0.18, 0.39, 0.15], 0.97, 0.04, 1.8),
        Material::Limestone => profile([0.58, 0.55, 0.44], [0.73, 0.68, 0.53], 0.82, 0.10, 3.1),
        Material::RedSand => profile([0.58, 0.19, 0.075], [0.70, 0.27, 0.10], 0.94, 0.05, 2.4),
        Material::Water => profile([0.02, 0.22, 0.30], [0.04, 0.34, 0.40], 0.12, 0.04, 1.0),
        Material::GlowCrystal => profile([0.12, 0.58, 0.78], [0.48, 0.94, 1.0], 0.30, 0.08, 5.0),
    }
}

const fn profile(
    base_srgb: [f32; 3],
    accent_srgb: [f32; 3],
    roughness: f32,
    roughness_variation: f32,
    normal_strength: f32,
) -> MaterialProfile {
    MaterialProfile {
        base_srgb,
        accent_srgb,
        roughness,
        roughness_variation,
        normal_strength,
    }
}

fn material_height(material: Material, u: f32, v: f32) -> f32 {
    let broad = periodic_noise(material, u, v, 8, 0x7a11);
    let fine = periodic_noise(material, u, v, 32, 0xc43b);
    (broad * 0.58 + fine * 0.24 + material_pattern(material, u, v) * 0.18).clamp(0.0, 1.0)
}

fn material_pattern(material: Material, u: f32, v: f32) -> f32 {
    let tau = std::f32::consts::TAU;
    let wave = |frequency: f32, phase: f32| (frequency * tau + phase).sin() * 0.5 + 0.5;
    match material {
        Material::Grass | Material::Moss => periodic_noise(material, u, v, 24, 0x101),
        Material::Stone | Material::Limestone | Material::GlowCrystal => {
            let seam = (periodic_noise(material, u, v, 8, 0x202) - 0.5).abs() * 2.0;
            (1.0 - seam).powi(5)
        }
        Material::Sand | Material::RedSand => wave(
            u * 7.0 + v * 2.0,
            periodic_noise(material, u, v, 4, 0x303) * 2.6,
        ),
        Material::Snow => wave(u * 11.0 - v * 9.0, 0.0).powi(8),
        Material::Clay => wave(v * 9.0, periodic_noise(material, u, v, 4, 0x404) * 1.4),
        Material::Basalt => {
            let cells = periodic_noise(material, u, v, 12, 0x505);
            ((cells - 0.5).abs() * 2.0).powi(4)
        }
        Material::Wood => wave(u * 14.0, periodic_noise(material, u, v, 4, 0x606) * 3.0),
        Material::Leaves => periodic_noise(material, u, v, 24, 0x707).powi(2),
        Material::Dirt | Material::Air | Material::Water => {
            periodic_noise(material, u, v, 16, 0x808)
        }
    }
}

fn periodic_noise(material: Material, u: f32, v: f32, cells: i32, salt: u32) -> f32 {
    let x = u * cells as f32;
    let y = v * cells as f32;
    let x0 = x.floor() as i32;
    let y0 = y.floor() as i32;
    let tx = smooth(x - x.floor());
    let ty = smooth(y - y.floor());
    let sample = |dx: i32, dy: i32| {
        hash_unit(
            material.id(),
            (x0 + dx).rem_euclid(cells),
            (y0 + dy).rem_euclid(cells),
            salt,
        )
    };
    let a = sample(0, 0);
    let b = sample(1, 0);
    let c = sample(0, 1);
    let d = sample(1, 1);
    lerp(lerp(a, b, tx), lerp(c, d, tx), ty)
}

fn hash_unit(material: u16, x: i32, y: i32, salt: u32) -> f32 {
    let mut value = u32::from(material).wrapping_mul(0x9e37_79b9) ^ salt;
    value ^= (x as u32).wrapping_mul(0x85eb_ca6b);
    value ^= (y as u32).wrapping_mul(0xc2b2_ae35);
    value ^= value >> 16;
    value = value.wrapping_mul(0x7feb_352d);
    value ^= value >> 15;
    value = value.wrapping_mul(0x846c_a68b);
    value ^= value >> 16;
    value as f32 / u32::MAX as f32
}

fn smooth(value: f32) -> f32 {
    value * value * (3.0 - 2.0 * value)
}

fn lerp(left: f32, right: f32, amount: f32) -> f32 {
    left + (right - left) * amount
}

fn srgb_to_linear(value: f32) -> f32 {
    if value <= 0.04045 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

fn linear_to_srgb(value: f32) -> f32 {
    if value <= 0.003_130_8 {
        value * 12.92
    } else {
        1.055 * value.powf(1.0 / 2.4) - 0.055
    }
}

fn encode_unorm(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atlas_layout_and_checksum_are_stable() {
        let first = MaterialDetailAtlas::generate();
        let second = MaterialDetailAtlas::generate();
        assert_eq!(first.mips, second.mips);
        assert_eq!(first.mips.len(), MATERIAL_DETAIL_MIP_COUNT as usize);
        assert_eq!(first.byte_len(), 2_621_400);
        assert_eq!(atlas_checksum(&first), 0xbf4f_305f_4694_6b25);
    }

    #[test]
    fn every_stable_material_id_maps_to_one_array_layer() {
        assert_eq!(MATERIAL_DETAIL_LAYER_COUNT, 15);
        for (layer, material) in Material::ALL.into_iter().enumerate() {
            assert_eq!(usize::from(material.id()), layer);
        }
    }

    #[test]
    fn material_detail_sampling_preserves_close_pixels_and_filters_distance() {
        let sampler = material_detail_sampler_descriptor();
        assert_eq!(sampler.mag_filter, FilterMode::Nearest);
        assert_eq!(sampler.min_filter, FilterMode::Linear);
        assert_eq!(sampler.mipmap_filter, MipmapFilterMode::Linear);
        assert_eq!(sampler.anisotropy_clamp, 1);
    }

    #[test]
    fn aggregate_terrain_profiles_are_rough_dielectrics() {
        for material in [
            Material::Grass,
            Material::Dirt,
            Material::Stone,
            Material::Sand,
            Material::Snow,
            Material::Clay,
            Material::Basalt,
            Material::Wood,
            Material::Leaves,
            Material::Moss,
            Material::Limestone,
            Material::RedSand,
        ] {
            let profile = material_profile(material);
            assert!(
                profile.roughness >= 0.80,
                "{material:?} is implausibly glossy at {}",
                profile.roughness
            );
            assert!(profile.roughness + profile.roughness_variation * 0.5 <= 1.0);
        }
        assert!(material_profile(Material::Water).roughness < 0.20);
        assert!(material_profile(Material::GlowCrystal).roughness < 0.40);
    }

    #[test]
    fn every_voxel_face_quantizes_to_exactly_three_world_aligned_texels() {
        let texel_width = voxels_world::VOXEL_SIZE_METRES / MATERIAL_TEXELS_PER_VOXEL as f32;
        assert!((texel_width - 1.0 / 30.0).abs() < f32::EPSILON);

        for voxel in -8..=8 {
            let start = voxel * MATERIAL_TEXELS_PER_VOXEL;
            for texel in 0..MATERIAL_TEXELS_PER_VOXEL {
                let centre_metres = (start + texel) as f32 * texel_width + texel_width * 0.5;
                assert_eq!(material_texel_index(centre_metres), start + texel);
            }
            let next_face = (voxel + 1) as f32 * voxels_world::VOXEL_SIZE_METRES;
            assert_eq!(
                material_texel_index(next_face - texel_width * 0.01),
                start + MATERIAL_TEXELS_PER_VOXEL - 1
            );
            assert_eq!(
                material_texel_index(next_face),
                start + MATERIAL_TEXELS_PER_VOXEL
            );
        }

        let shader = include_str!("shaders/voxels.wgsl");
        assert!(shader.contains("const MATERIAL_TEXELS_PER_VOXEL: f32 = 3.0;"));
        assert!(shader.contains("textureSampleGrad"));
    }

    fn material_texel_index(world_metres: f32) -> i32 {
        (world_metres * MATERIAL_TEXELS_PER_VOXEL as f32 / voxels_world::VOXEL_SIZE_METRES + 0.0001)
            .floor() as i32
    }

    #[test]
    fn mip_chain_reaches_one_texel_and_keeps_normals_bounded() {
        let atlas = MaterialDetailAtlas::generate();
        for (level, mip) in atlas.mips.iter().enumerate() {
            let expected_width = MATERIAL_DETAIL_SIZE >> level;
            let expected_bytes = expected_width as usize
                * expected_width as usize
                * MATERIAL_DETAIL_LAYER_COUNT as usize
                * 4;
            assert_eq!(mip.width, expected_width);
            assert_eq!(mip.albedo.len(), expected_bytes);
            assert_eq!(mip.normal_roughness.len(), expected_bytes);
            for texel in mip.normal_roughness.chunks_exact(4) {
                let normal = glam::Vec3::new(
                    texel[0] as f32 / 255.0 * 2.0 - 1.0,
                    texel[1] as f32 / 255.0 * 2.0 - 1.0,
                    texel[2] as f32 / 255.0 * 2.0 - 1.0,
                );
                assert!(normal.is_finite());
                assert!(normal.length() <= 1.02);
                assert!(normal.z > 0.0);
            }
        }
        assert_eq!(atlas.mips.last().map(|mip| mip.width), Some(1));
    }

    #[test]
    fn downsampling_averages_linear_albedo_and_preserves_normal_variance() {
        let source = LayerTexels {
            albedo: vec![[0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            normal_roughness: vec![
                [0.0, 0.0, 1.0, 0.5],
                [0.6, 0.0, 0.8, 0.5],
                [-0.6, 0.0, 0.8, 0.5],
                [0.0, 0.6, 0.8, 0.5],
            ],
        };
        let downsampled = downsample_layer(&source, 2);
        assert_eq!(downsampled.albedo, vec![[0.25, 0.25, 0.25]]);
        let normal = downsampled.normal_roughness[0];
        assert!(glam::Vec3::new(normal[0], normal[1], normal[2]).length() < 1.0);
        assert!((normal[3] - 0.5).abs() < 0.0001);
    }

    #[test]
    fn procedural_signals_repeat_at_texture_boundaries() {
        for material in Material::ALL {
            for [u, v] in [[0.13, 0.47], [0.92, -0.26], [-0.38, 1.41]] {
                let reference = material_height(material, u, v);
                assert!((reference - material_height(material, u + 1.0, v)).abs() < 0.0001);
                assert!((reference - material_height(material, u, v + 1.0)).abs() < 0.0001);
            }
        }
    }

    fn atlas_checksum(atlas: &MaterialDetailAtlas) -> u64 {
        atlas.mips.iter().fold(0xcbf2_9ce4_8422_2325, |hash, mip| {
            mip.albedo
                .iter()
                .chain(&mip.normal_roughness)
                .fold(hash, |hash, byte| {
                    (hash ^ u64::from(*byte)).wrapping_mul(0x100_0000_01b3)
                })
        })
    }
}
