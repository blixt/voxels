#[cfg(not(target_os = "macos"))]
compile_error!("voxels-world-source with terrain-metal requires macOS and Apple Metal");

use std::collections::BTreeMap;
use std::path::PathBuf;
use voxels_world::{
    SurfaceSampleBlockRequest, WorldProduct, WorldProductBatch, WorldProductPriority,
    WorldProductRequest,
};
use voxels_world_service::{LoadedWorldServiceConfig, WorldSourceMode};

#[allow(
    clippy::print_stdout,
    reason = "this diagnostic service bootstrap reports its selected source and sample"
)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config_path = std::env::args_os()
        .nth(1)
        .map_or_else(|| PathBuf::from("config/world-service.toml"), PathBuf::from);
    let loaded = LoadedWorldServiceConfig::load(&config_path)?;
    let sample_origin = match loaded.config().source {
        WorldSourceMode::ProceduralV16 => [0, 0],
        WorldSourceMode::TerrainDiffusion30m => {
            loaded.config().terrain_diffusion.world_origin_voxels
        }
    };
    let model_root = (loaded.config().source == WorldSourceMode::TerrainDiffusion30m)
        .then(|| loaded.terrain_model_root())
        .transpose()?;
    let source = loaded.build_world_source()?;
    let identity = source.identity();
    const SAMPLE_EDGE: i32 = 16;
    let sample_spacing_voxels = match loaded.config().source {
        WorldSourceMode::ProceduralV16 => 2_400,
        WorldSourceMode::TerrainDiffusion30m => {
            2_400 * loaded.config().terrain_diffusion.horizontal_scale as i32
        }
    };
    let requests = (0..SAMPLE_EDGE)
        .flat_map(|z| {
            (0..SAMPLE_EDGE).map(move |x| {
                WorldProductRequest::SurfaceSampleBlock(SurfaceSampleBlockRequest {
                    origin: [
                        sample_origin[0] + x * sample_spacing_voxels,
                        sample_origin[1] + z * sample_spacing_voxels,
                    ],
                    sample_shape: [1, 1],
                })
            })
        })
        .collect::<Vec<_>>();
    let result = source.generate_batch(WorldProductBatch {
        priority: WorldProductPriority::VisibleSurface,
        requests,
    })?;
    let mut heights = Vec::with_capacity(result.items.len());
    let mut temperatures = Vec::with_capacity(result.items.len());
    let mut moistures = Vec::with_capacity(result.items.len());
    let mut ridges = Vec::with_capacity(result.items.len());
    let mut materials = BTreeMap::<u16, usize>::new();
    let mut regions = BTreeMap::<u8, usize>::new();
    for item in result.items {
        let WorldProduct::SurfaceSampleBlock(block) = item.result? else {
            return Err(
                std::io::Error::other("configured source returned the wrong product").into(),
            );
        };
        let sample = block
            .samples()
            .first()
            .ok_or_else(|| std::io::Error::other("configured source returned an empty sample"))?;
        heights.push(sample.height);
        temperatures.push(sample.temperature);
        moistures.push(sample.moisture);
        ridges.push(sample.ridge);
        *materials.entry(sample.material.id()).or_default() += 1;
        *regions.entry(sample.region as u8).or_default() += 1;
    }

    println!("config={}", config_path.display());
    println!("source={:?}", identity.source_kind);
    println!("source_identity={}", identity.identity_hash());
    if let Some(model_root) = model_root {
        println!("model_root={}", model_root.display());
    }
    println!("sample_origin={},{}", sample_origin[0], sample_origin[1]);
    println!("sample_count={}", heights.len());
    println!(
        "sample_height_voxels_min={} sample_height_voxels_max={}",
        heights.iter().copied().min().unwrap_or_default(),
        heights.iter().copied().max().unwrap_or_default(),
    );
    println!(
        "sample_temperature_min={:.3} sample_temperature_max={:.3}",
        minimum(&temperatures),
        maximum(&temperatures),
    );
    println!(
        "sample_moisture_min={:.3} sample_moisture_max={:.3}",
        minimum(&moistures),
        maximum(&moistures),
    );
    println!(
        "sample_ridge_min={:.3} sample_ridge_max={:.3}",
        minimum(&ridges),
        maximum(&ridges),
    );
    println!("sample_material_histogram={materials:?}");
    println!("sample_region_histogram={regions:?}");
    Ok(())
}

fn minimum(values: &[f32]) -> f32 {
    values.iter().copied().fold(f32::INFINITY, f32::min)
}

fn maximum(values: &[f32]) -> f32 {
    values.iter().copied().fold(f32::NEG_INFINITY, f32::max)
}
