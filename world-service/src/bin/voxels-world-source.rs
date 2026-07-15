#[cfg(not(target_os = "macos"))]
compile_error!("voxels-world-source with terrain-metal requires macOS and Apple Metal");

use std::collections::BTreeMap;
use std::path::PathBuf;
use voxels_world::{
    SurfaceSampleBlockRequest, TreeSpecies, WorldProduct, WorldProductBatch, WorldProductPriority,
    WorldProductRequest, WorldSourceEngine,
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
    let ecology_survey = std::env::args_os()
        .nth(2)
        .is_some_and(|argument| argument == "--ecology-survey");
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
    if ecology_survey {
        print_ecology_survey(source.as_ref(), loaded.config().spawn.xz_voxels);
    }
    Ok(())
}

#[allow(
    clippy::print_stdout,
    reason = "this explicit diagnostic prints its density survey for human inspection"
)]
fn print_ecology_survey(source: &dyn WorldSourceEngine, centre: [i32; 2]) {
    const WINDOW_EDGE_VOXELS: i32 = 1_280;
    const GRID_EDGE: i32 = 25;
    let half_grid = GRID_EDGE / 2;
    let mut windows = Vec::with_capacity((GRID_EDGE * GRID_EDGE) as usize);
    let mut species_histogram = BTreeMap::<TreeSpecies, usize>::new();

    println!("ecology_map_legend=space:0 .:1-4 ::5-15 +:16-47 *:48-119 #:120+");
    for grid_z in 0..GRID_EDGE {
        let mut row = Vec::with_capacity(GRID_EDGE as usize);
        for grid_x in 0..GRID_EDGE {
            let origin = [
                centre[0] + (grid_x - half_grid) * WINDOW_EDGE_VOXELS - WINDOW_EDGE_VOXELS / 2,
                centre[1] + (grid_z - half_grid) * WINDOW_EDGE_VOXELS - WINDOW_EDGE_VOXELS / 2,
            ];
            let mut local_species = BTreeMap::<TreeSpecies, usize>::new();
            for species in source
                .skyline_features_anchored_in([
                    origin,
                    [
                        origin[0] + WINDOW_EDGE_VOXELS,
                        origin[1] + WINDOW_EDGE_VOXELS,
                    ],
                ])
                .into_iter()
                .filter_map(|feature| feature.tree_species())
            {
                *local_species.entry(species).or_default() += 1;
                *species_histogram.entry(species).or_default() += 1;
            }
            let count = local_species.values().sum();
            row.push(count);
            windows.push((count, origin, local_species));
        }
        let map = row
            .into_iter()
            .map(ecology_density_character)
            .collect::<String>();
        println!("ecology_map={map}");
    }

    windows.sort_by_key(|(count, _, _)| *count);
    let percentile = |percent: usize| {
        let index = (windows.len() - 1) * percent / 100;
        windows[index].0
    };
    println!(
        "ecology_windows={} ecology_window_metres={} ecology_trees_min={} ecology_trees_p10={} ecology_trees_median={} ecology_trees_p90={} ecology_trees_max={}",
        windows.len(),
        WINDOW_EDGE_VOXELS / 10,
        windows.first().map_or(0, |window| window.0),
        percentile(10),
        percentile(50),
        percentile(90),
        windows.last().map_or(0, |window| window.0),
    );
    println!("ecology_species_histogram={species_histogram:?}");
    for (rank, (count, origin, species)) in windows.iter().rev().take(5).enumerate() {
        println!(
            "ecology_dense_rank={} origin_voxels={},{} trees={} species={species:?}",
            rank + 1,
            origin[0],
            origin[1],
            count,
        );
    }
}

const fn ecology_density_character(count: usize) -> char {
    match count {
        0 => ' ',
        1..=4 => '.',
        5..=15 => ':',
        16..=47 => '+',
        48..=119 => '*',
        _ => '#',
    }
}

fn minimum(values: &[f32]) -> f32 {
    values.iter().copied().fold(f32::INFINITY, f32::min)
}

fn maximum(values: &[f32]) -> f32 {
    values.iter().copied().fold(f32::NEG_INFINITY, f32::max)
}
