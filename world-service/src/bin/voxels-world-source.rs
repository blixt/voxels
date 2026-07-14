#[cfg(not(target_os = "macos"))]
compile_error!("voxels-world-source with terrain-metal requires macOS and Apple Metal");

use std::path::PathBuf;
use voxels_world::{MacroBlockBatch, MacroBlockRequest, WorldProductPriority};
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
    let source = loaded.build_macro_source()?;
    let identity = source.identity();
    let result = source.request_blocks(MacroBlockBatch {
        priority: WorldProductPriority::VisibleSurface,
        requests: vec![MacroBlockRequest {
            origin: sample_origin,
            sample_shape: [1, 1],
            stride_voxels: 300,
        }],
    })?;
    let block = result
        .blocks
        .first()
        .ok_or_else(|| std::io::Error::other("configured source returned no macro block"))?;
    let elevation_voxels = block
        .elevation_voxels
        .first()
        .ok_or_else(|| std::io::Error::other("configured source returned an empty macro block"))?;
    let valid = block.validity.first().copied().unwrap_or(false);

    println!("config={}", config_path.display());
    println!("source={:?}", identity.source_kind);
    println!("source_identity={}", identity.identity_hash());
    if let Some(model_root) = model_root {
        println!("model_root={}", model_root.display());
    }
    println!("sample_origin={},{}", sample_origin[0], sample_origin[1]);
    println!("sample_elevation_voxels={elevation_voxels:.3}");
    println!("sample_valid={valid}");
    Ok(())
}
