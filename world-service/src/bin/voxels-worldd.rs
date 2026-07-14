use std::path::PathBuf;
use voxels_world_service::{LoadedWorldServiceConfig, serve_loaded_config};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config_path = std::env::args_os()
        .nth(1)
        .map_or_else(|| PathBuf::from("config/world-service.toml"), PathBuf::from);
    let loaded = LoadedWorldServiceConfig::load(config_path)?;
    serve_loaded_config(&loaded).await?;
    Ok(())
}
