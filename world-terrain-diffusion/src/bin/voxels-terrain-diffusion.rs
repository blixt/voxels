#[cfg(not(target_os = "macos"))]
compile_error!("voxels-terrain-diffusion requires macOS and Apple Metal");

use std::{collections::VecDeque, path::PathBuf};
use voxels_world_terrain_diffusion::{
    DetailTile, MetalTerrainDiffusion, TerrainDiffusionConfig, TerrainPrecision, fetch_pinned_model,
};

#[allow(
    clippy::print_stdout,
    reason = "this diagnostic CLI reports model and benchmark results"
)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut arguments = std::env::args_os().skip(1);
    let command = arguments
        .next()
        .and_then(|value| value.into_string().ok())
        .unwrap_or_else(|| "smoke".to_owned());
    let cache = arguments.next().map_or_else(default_cache, PathBuf::from);
    let model_root = if command == "fetch" || !cache.join("config.json").is_file() {
        fetch_pinned_model(&cache)?
    } else {
        cache
    };
    if command == "fetch" {
        println!("{}", model_root.display());
        return Ok(());
    }
    if command != "smoke"
        && command != "base-smoke"
        && command != "detail-smoke"
        && command != "detail-survey"
        && command != "survey-smoke"
        && command != "counterproof"
    {
        return Err(format!(
            "unknown command {command}; expected fetch, smoke, base-smoke, detail-smoke, detail-survey, survey-smoke, or counterproof"
        )
        .into());
    }
    let seed = std::env::var("VOXELS_TERRAIN_SEED")
        .ok()
        .map(|value| value.parse::<u64>())
        .transpose()?
        .unwrap_or(0x5eed_cafe);
    let latent_window = std::env::var("VOXELS_TERRAIN_WINDOW")
        .ok()
        .map(|value| parse_coordinate_pair(&value))
        .transpose()?
        .unwrap_or([-2, -1]);
    let config = TerrainDiffusionConfig {
        model_root,
        seed,
        precision: TerrainPrecision::Float16,
        require_metal: true,
        world_origin_voxels: [0, 0],
        horizontal_scale: 1,
        latent_window,
        quality_histogram: [0.0, 0.0, 0.0, 1.0, 1.5],
    };
    let runtime =
        if command == "detail-smoke" || command == "detail-survey" || command == "counterproof" {
            MetalTerrainDiffusion::load_full(config)?
        } else if command == "base-smoke" || command == "survey-smoke" {
            MetalTerrainDiffusion::load_base(config)?
        } else {
            MetalTerrainDiffusion::load_coarse(config)?
        };
    if command == "counterproof" {
        let baseline = vec![0.0_f32; 11 * 64 * 64];
        let mut conditioned = baseline.clone();
        conditioned[6 * 64 * 64..7 * 64 * 64].fill(1.0);
        let scalar_conditioning = [(0.5_f32 / 8.0).ln(); 5];
        let baseline = runtime.coarse_forward(&baseline, 0.5, scalar_conditioning)?;
        let conditioned = runtime.coarse_forward(&conditioned, 0.5, scalar_conditioning)?;
        let maximum_delta = baseline
            .values
            .iter()
            .zip(conditioned.values)
            .map(|(left, right)| (left - right).abs())
            .fold(0.0_f32, f32::max);
        let mut base_conditioning = vec![1.0_f32; 7 * 4 * 4];
        for (channel, mean) in [14.99, 11.65, 15.87, 619.26, 833.12, 69.40]
            .into_iter()
            .enumerate()
        {
            base_conditioning[channel * 16..(channel + 1) * 16].fill(mean);
        }
        let base_baseline = runtime.generate_latent_tile([0, 0], &base_conditioning)?;
        base_conditioning[..16].fill(35.0);
        let base_conditioned = runtime.generate_latent_tile([0, 0], &base_conditioning)?;
        let maximum_base_delta = base_baseline
            .values
            .iter()
            .zip(base_conditioned.values)
            .map(|(left, right)| (left - right).abs())
            .fold(0.0_f32, f32::max);
        let maximum_base_forward_delta = runtime.base_forward_conditioning_delta()?;
        let decoder_baseline = runtime.decode_detail_tile([0, 0], 128, &[0.0; 5 * 16 * 16])?;
        let mut decoder_conditioning = vec![0.0_f32; 5 * 16 * 16];
        decoder_conditioning[..4 * 16 * 16].fill(1.0);
        let decoder_conditioned = runtime.decode_detail_tile([0, 0], 128, &decoder_conditioning)?;
        let maximum_decoder_delta = decoder_baseline
            .residual_sqrt_elevation
            .iter()
            .zip(decoder_conditioned.residual_sqrt_elevation)
            .map(|(left, right)| (left - right).abs())
            .fold(0.0_f32, f32::max);
        println!("device={}", runtime.device_description());
        println!("precision={:?}", runtime.precision());
        println!("maximum_coarse_conditioning_delta={maximum_delta:.9}");
        println!("maximum_base_conditioning_delta={maximum_base_delta:.9}");
        println!("maximum_base_forward_delta={maximum_base_forward_delta:.9}");
        println!("maximum_decoder_conditioning_delta={maximum_decoder_delta:.9}");
        if maximum_delta <= 1.0e-4
            || maximum_base_delta <= 1.0e-4
            || maximum_base_forward_delta <= 1.0e-4
            || maximum_decoder_delta <= 1.0e-4
        {
            return Err("a learned stage ignored changed image conditioning".into());
        }
        return Ok(());
    }
    if command == "detail-smoke" {
        let generated = runtime.generate_full_detail_tile(latent_window)?;
        let coarse_pixels = generated.coarse.edge * generated.coarse.edge;
        let coarse_elevation = &generated.coarse.values[..coarse_pixels];
        let coarse_minimum = coarse_elevation
            .iter()
            .copied()
            .fold(f32::INFINITY, f32::min);
        let coarse_maximum = coarse_elevation
            .iter()
            .copied()
            .fold(f32::NEG_INFINITY, f32::max);
        let coarse_land_fraction = coarse_elevation
            .iter()
            .filter(|value| **value > 0.0)
            .count() as f64
            / coarse_pixels as f64;
        let [coarse_patch_z, coarse_patch_x] = generated.latent.coarse_patch_origin;
        let coarse_patch_mean = (0..4)
            .flat_map(|z| {
                (0..4).map(move |x| {
                    coarse_elevation
                        [(coarse_patch_z + z) * generated.coarse.edge + coarse_patch_x + x]
                })
            })
            .map(f64::from)
            .sum::<f64>()
            / 16.0;
        let latent_pixels = generated.latent.edge * generated.latent.edge;
        let latent_low_frequency = &generated.latent.values[4 * latent_pixels..5 * latent_pixels];
        let latent_low_minimum = latent_low_frequency
            .iter()
            .copied()
            .fold(f32::INFINITY, f32::min);
        let latent_low_maximum = latent_low_frequency
            .iter()
            .copied()
            .fold(f32::NEG_INFINITY, f32::max);
        let minimum = generated
            .detail
            .elevation_metres
            .iter()
            .copied()
            .fold(f32::INFINITY, f32::min);
        let maximum = generated
            .detail
            .elevation_metres
            .iter()
            .copied()
            .fold(f32::NEG_INFINITY, f32::max);
        let mean = generated
            .detail
            .elevation_metres
            .iter()
            .map(|value| f64::from(*value))
            .sum::<f64>()
            / generated.detail.elevation_metres.len() as f64;
        let detail_edge = generated.detail.edge;
        let detail_resolution = generated.detail.horizontal_resolution_metres as f32;
        let mut ordered_elevation = generated.detail.elevation_metres.clone();
        ordered_elevation.sort_by(f32::total_cmp);
        let elevation_p05 = percentile(&ordered_elevation, 0.05);
        let elevation_p50 = percentile(&ordered_elevation, 0.50);
        let elevation_p95 = percentile(&ordered_elevation, 0.95);
        let detail_land_fraction = ordered_elevation.partition_point(|value| *value <= 0.0) as f64;
        let detail_land_fraction = 1.0 - detail_land_fraction / ordered_elevation.len() as f64;
        let center_elevation =
            generated.detail.elevation_metres[(detail_edge / 2) * detail_edge + detail_edge / 2];
        let mut slopes = Vec::with_capacity((detail_edge - 2) * (detail_edge - 2));
        for row in 1..detail_edge - 1 {
            for column in 1..detail_edge - 1 {
                let horizontal = (generated.detail.elevation_metres
                    [row * detail_edge + column + 1]
                    - generated.detail.elevation_metres[row * detail_edge + column - 1])
                    / (2.0 * detail_resolution);
                let vertical = (generated.detail.elevation_metres
                    [(row + 1) * detail_edge + column]
                    - generated.detail.elevation_metres[(row - 1) * detail_edge + column])
                    / (2.0 * detail_resolution);
                slopes.push(horizontal.hypot(vertical).atan().to_degrees());
            }
        }
        slopes.sort_by(f32::total_cmp);
        let mut kilometre_relief = Vec::new();
        for block_row in (0..detail_edge).step_by(32) {
            for block_column in (0..detail_edge).step_by(32) {
                let mut block_minimum = f32::INFINITY;
                let mut block_maximum = f32::NEG_INFINITY;
                for row in block_row..(block_row + 32).min(detail_edge) {
                    for column in block_column..(block_column + 32).min(detail_edge) {
                        let value = generated.detail.elevation_metres[row * detail_edge + column];
                        block_minimum = block_minimum.min(value);
                        block_maximum = block_maximum.max(value);
                    }
                }
                kilometre_relief.push(block_maximum - block_minimum);
            }
        }
        kilometre_relief.sort_by(f32::total_cmp);
        if let Some(path) = std::env::var_os("VOXELS_TERRAIN_PREVIEW") {
            write_hillshade_preview(
                PathBuf::from(path),
                &generated.detail.elevation_metres,
                detail_edge,
                detail_resolution,
            )?;
        }
        println!("device={}", runtime.device_description());
        println!("precision={:?}", runtime.precision());
        println!("seed={seed}");
        println!(
            "coarse_elevation_sqrt_min={coarse_minimum:.3} coarse_elevation_sqrt_max={coarse_maximum:.3} coarse_land_fraction={coarse_land_fraction:.3}"
        );
        println!(
            "coarse_patch_elevation_sqrt_mean={coarse_patch_mean:.3} latent_low_min={latent_low_minimum:.3} latent_low_max={latent_low_maximum:.3}"
        );
        println!(
            "coarse_ms={:.3} latent_ms={:.3} decoder_ms={:.3} coarse_patch={:?} latent_patch={:?}",
            generated.coarse.elapsed_seconds * 1_000.0,
            generated.latent.elapsed_seconds * 1_000.0,
            generated.detail.elapsed_seconds * 1_000.0,
            generated.latent.coarse_patch_origin,
            generated.latent_patch_origin,
        );
        println!(
            "detail={}x{} resolution_m={} min_m={minimum:.3} max_m={maximum:.3} mean_m={mean:.3}",
            generated.detail.edge,
            generated.detail.edge,
            generated.detail.horizontal_resolution_metres,
        );
        println!(
            "detail_p05_m={elevation_p05:.3} detail_p50_m={elevation_p50:.3} detail_p95_m={elevation_p95:.3} center_m={center_elevation:.3} land_fraction={detail_land_fraction:.3}"
        );
        println!(
            "slope_p50_degrees={:.3} slope_p90_degrees={:.3} relief_960m_p50={:.3} relief_960m_p90={:.3}",
            percentile(&slopes, 0.50),
            percentile(&slopes, 0.90),
            percentile(&kilometre_relief, 0.50),
            percentile(&kilometre_relief, 0.90),
        );
        return Ok(());
    }
    if command == "detail-survey" {
        let radius = survey_radius(1, 3)?;
        let mut candidates = Vec::new();
        let first_row = latent_window[0].saturating_sub(radius);
        let last_row = latent_window[0].saturating_add(radius);
        let first_column = latent_window[1].saturating_sub(radius);
        let last_column = latent_window[1].saturating_add(radius);
        for row in first_row..=last_row {
            for column in first_column..=last_column {
                let generated = runtime.generate_full_detail_tile([row, column])?;
                candidates.push(rank_detail_window(
                    [row, column],
                    generated.detail_model_origin,
                    &generated.detail,
                    generated.coarse.elapsed_seconds,
                    generated.latent.elapsed_seconds,
                )?);
            }
        }
        candidates.sort_by(|left, right| right.score.total_cmp(&left.score));
        println!("device={}", runtime.device_description());
        println!("precision={:?}", runtime.precision());
        println!("seed={seed} center={latent_window:?} radius={radius}");
        for (rank, candidate) in candidates.into_iter().enumerate() {
            println!(
                "rank={} latent_window={:?} score={:.3} mountain={:.3} coast={:.3} valley={:.3} playable={:.3} land_fraction={:.3} elevation_p05_m={:.1} elevation_p50_m={:.1} elevation_p95_m={:.1} relief_960m_p90_m={:.1} slope_p90_degrees={:.1} coastline_km_per_100km2={:.2} coastal_relief_p90_m={:.1} sea_inlet_depth_m={:.0} narrow_sea_fraction={:.3} valley_depth_p95_m={:.1} valley_fraction={:.3} center_elevation_m={:.1} center_slope_degrees={:.1} spawn_model={:?} spawn_offset_m={:?} spawn_elevation_m={:.1} spawn_slope_degrees={:.1} spawn_relief_240m_m={:.1} spawn_sea_distance_m={:.0} coarse_ms={:.1} latent_ms={:.1} decoder_ms={:.1}",
                rank + 1,
                candidate.latent_window,
                candidate.score,
                candidate.mountain_score,
                candidate.coast_score,
                candidate.valley_score,
                candidate.spawn.score,
                candidate.land_fraction,
                candidate.elevation_p05_metres,
                candidate.elevation_p50_metres,
                candidate.elevation_p95_metres,
                candidate.relief_960m_p90_metres,
                candidate.slope_p90_degrees,
                candidate.coastline_km_per_100_square_km,
                candidate.coastal_relief_p90_metres,
                candidate.sea_inlet_depth_metres,
                candidate.narrow_sea_fraction,
                candidate.valley_depth_p95_metres,
                candidate.valley_fraction,
                candidate.center_elevation_metres,
                candidate.center_slope_degrees,
                candidate.spawn.model_coordinate,
                candidate.spawn.offset_metres,
                candidate.spawn.elevation_metres,
                candidate.spawn.slope_degrees,
                candidate.spawn.local_relief_metres,
                candidate.spawn.sea_distance_metres,
                candidate.coarse_seconds * 1_000.0,
                candidate.latent_seconds * 1_000.0,
                candidate.decoder_seconds * 1_000.0,
            );
        }
        return Ok(());
    }
    if command == "survey-smoke" {
        let radius = survey_radius(2, 8)?;
        let mut candidates = Vec::new();
        let first_row = latent_window[0].saturating_sub(radius);
        let last_row = latent_window[0].saturating_add(radius);
        let first_column = latent_window[1].saturating_sub(radius);
        let last_column = latent_window[1].saturating_add(radius);
        for row in first_row..=last_row {
            for column in first_column..=last_column {
                let (coarse, latent) = runtime.generate_coarse_and_latent([row, column])?;
                let pixels = latent.edge * latent.edge;
                let low = &latent.values[4 * pixels..5 * pixels];
                let mean =
                    low.iter().map(|value| f64::from(*value)).sum::<f64>() / low.len() as f64;
                let variance = low
                    .iter()
                    .map(|value| (f64::from(*value) - mean).powi(2))
                    .sum::<f64>()
                    / low.len() as f64;
                let minimum = low.iter().copied().fold(f32::INFINITY, f32::min);
                let maximum = low.iter().copied().fold(f32::NEG_INFINITY, f32::max);
                let context_origin = latent.coarse_patch_origin;
                let mut coarse_context_sum = 0.0_f64;
                let mut coarse_context_land = 0_usize;
                for context_row in 0..4 {
                    for context_column in 0..4 {
                        let value = coarse.values[(context_origin[0] + context_row) * coarse.edge
                            + context_origin[1]
                            + context_column];
                        coarse_context_sum += f64::from(value);
                        coarse_context_land += usize::from(value > 0.0);
                    }
                }
                candidates.push((
                    variance.sqrt(),
                    [row, column],
                    minimum,
                    maximum,
                    coarse_context_sum / 16.0,
                    coarse_context_land as f64 / 16.0,
                    coarse.elapsed_seconds,
                    latent.elapsed_seconds,
                ));
            }
        }
        candidates.sort_by(|left, right| right.0.total_cmp(&left.0));
        println!("device={}", runtime.device_description());
        println!("precision={:?}", runtime.precision());
        println!("seed={seed} center={latent_window:?} radius={radius}");
        for (
            rank,
            (
                standard_deviation,
                window,
                minimum,
                maximum,
                coarse_context_mean,
                coarse_context_land,
                coarse_s,
                latent_s,
            ),
        ) in candidates.into_iter().enumerate()
        {
            println!(
                "rank={} latent_window={window:?} low_std={standard_deviation:.6} low_min={minimum:.6} low_max={maximum:.6} coarse_context_mean={coarse_context_mean:.3} coarse_context_land={coarse_context_land:.3} coarse_ms={:.3} latent_ms={:.3}",
                rank + 1,
                coarse_s * 1_000.0,
                latent_s * 1_000.0,
            );
        }
        return Ok(());
    }
    if command == "base-smoke" {
        let (coarse, latent) = runtime.generate_coarse_and_latent(latent_window)?;
        let minimum = latent.values.iter().copied().fold(f32::INFINITY, f32::min);
        let maximum = latent
            .values
            .iter()
            .copied()
            .fold(f32::NEG_INFINITY, f32::max);
        let mean = latent
            .values
            .iter()
            .map(|value| f64::from(*value))
            .sum::<f64>()
            / latent.values.len() as f64;
        println!("device={}", runtime.device_description());
        println!("precision={:?}", runtime.precision());
        println!("seed={seed}");
        println!(
            "coarse_ms={:.3} latent_ms={:.3} coarse_patch={:?}",
            coarse.elapsed_seconds * 1_000.0,
            latent.elapsed_seconds * 1_000.0,
            latent.coarse_patch_origin,
        );
        println!(
            "latent={}x{}x{} min={minimum:.6} max={maximum:.6} mean={mean:.6}",
            latent.channels, latent.edge, latent.edge,
        );
        return Ok(());
    }
    let conditioning = vec![0.0f32; 5 * 64 * 64];
    let tile = runtime.generate_coarse_tile([0, 0], &conditioning, [0.5; 5])?;
    let minimum = tile.values.iter().copied().fold(f32::INFINITY, f32::min);
    let maximum = tile
        .values
        .iter()
        .copied()
        .fold(f32::NEG_INFINITY, f32::max);
    let mean = tile
        .values
        .iter()
        .map(|value| f64::from(*value))
        .sum::<f64>()
        / tile.values.len() as f64;
    println!("device={}", runtime.device_description());
    println!("precision={:?}", runtime.precision());
    println!("seed={seed}");
    println!(
        "coarse={}x{}x{} elapsed_ms={:.3} min={minimum:.6} max={maximum:.6} mean={mean:.6}",
        tile.channels,
        tile.edge,
        tile.edge,
        tile.elapsed_seconds * 1_000.0
    );
    Ok(())
}

fn default_cache() -> PathBuf {
    std::env::var_os("HOME").map_or_else(
        || PathBuf::from(".cache/terrain-diffusion"),
        |home| PathBuf::from(home).join("Library/Caches/voxels/terrain-diffusion"),
    )
}

fn parse_coordinate_pair(value: &str) -> Result<[i32; 2], Box<dyn std::error::Error>> {
    let mut coordinates = value.split(',');
    let row = coordinates
        .next()
        .ok_or("missing terrain-window row")?
        .trim()
        .parse()?;
    let column = coordinates
        .next()
        .ok_or("missing terrain-window column")?
        .trim()
        .parse()?;
    if coordinates.next().is_some() {
        return Err("VOXELS_TERRAIN_WINDOW must contain exactly row,column".into());
    }
    Ok([row, column])
}

fn survey_radius(default: i32, maximum: i32) -> Result<i32, Box<dyn std::error::Error>> {
    let radius = std::env::var("VOXELS_TERRAIN_SURVEY_RADIUS")
        .ok()
        .map(|value| value.parse::<i32>())
        .transpose()?
        .unwrap_or(default);
    if !(0..=maximum).contains(&radius) {
        return Err(format!(
            "VOXELS_TERRAIN_SURVEY_RADIUS must be in 0..={maximum} for this command"
        )
        .into());
    }
    Ok(radius)
}

#[derive(Clone, Debug)]
struct SpawnRank {
    score: f32,
    model_coordinate: [i32; 2],
    offset_metres: [i32; 2],
    elevation_metres: f32,
    slope_degrees: f32,
    local_relief_metres: f32,
    sea_distance_metres: f32,
}

#[derive(Clone, Debug)]
struct DetailWindowRank {
    latent_window: [i32; 2],
    score: f32,
    mountain_score: f32,
    coast_score: f32,
    valley_score: f32,
    land_fraction: f32,
    elevation_p05_metres: f32,
    elevation_p50_metres: f32,
    elevation_p95_metres: f32,
    relief_960m_p90_metres: f32,
    slope_p90_degrees: f32,
    coastline_km_per_100_square_km: f32,
    coastal_relief_p90_metres: f32,
    sea_inlet_depth_metres: f32,
    narrow_sea_fraction: f32,
    valley_depth_p95_metres: f32,
    valley_fraction: f32,
    center_elevation_metres: f32,
    center_slope_degrees: f32,
    spawn: SpawnRank,
    coarse_seconds: f64,
    latent_seconds: f64,
    decoder_seconds: f64,
}

fn rank_detail_window(
    latent_window: [i32; 2],
    detail_model_origin: [i32; 2],
    detail: &DetailTile,
    coarse_seconds: f64,
    latent_seconds: f64,
) -> Result<DetailWindowRank, Box<dyn std::error::Error>> {
    let edge = detail.edge;
    if edge < 3 || detail.elevation_metres.len() != edge * edge {
        return Err(
            "detail ranker requires a square elevation tile at least 3 samples wide".into(),
        );
    }
    let resolution = detail.horizontal_resolution_metres as f32;
    let elevation = &detail.elevation_metres;
    let mut ordered_elevation = elevation.clone();
    ordered_elevation.sort_by(f32::total_cmp);
    let elevation_p05_metres = percentile(&ordered_elevation, 0.05);
    let elevation_p50_metres = percentile(&ordered_elevation, 0.50);
    let elevation_p95_metres = percentile(&ordered_elevation, 0.95);
    let land_fraction = 1.0
        - ordered_elevation.partition_point(|value| *value <= 0.0) as f32
            / ordered_elevation.len() as f32;

    let slopes = slope_degrees(elevation, edge, resolution);
    let mut ordered_slopes = slopes.clone();
    ordered_slopes.sort_by(f32::total_cmp);
    let slope_p90_degrees = percentile(&ordered_slopes, 0.90);

    let block_edge = ((960.0 / resolution).round() as usize).max(1);
    let mut block_relief = Vec::new();
    for block_row in (0..edge).step_by(block_edge) {
        for block_column in (0..edge).step_by(block_edge) {
            let mut minimum = f32::INFINITY;
            let mut maximum = f32::NEG_INFINITY;
            for row in block_row..(block_row + block_edge).min(edge) {
                for column in block_column..(block_column + block_edge).min(edge) {
                    let value = elevation[row * edge + column];
                    minimum = minimum.min(value);
                    maximum = maximum.max(value);
                }
            }
            block_relief.push(maximum - minimum);
        }
    }
    block_relief.sort_by(f32::total_cmp);
    let relief_960m_p90_metres = percentile(&block_relief, 0.90);

    let sea = boundary_connected_sea(elevation, edge);
    let sea_count = sea.iter().filter(|value| **value).count();
    let sea_distance = grid_distance(&sea, edge);
    let land = elevation
        .iter()
        .map(|value| *value > 0.0)
        .collect::<Vec<_>>();
    let land_distance = grid_distance(&land, edge);
    let mut coast_edges = 0_usize;
    let mut coastal_elevation = Vec::new();
    let mut sea_inlet_depth_samples = 0_usize;
    let mut narrow_sea = 0_usize;
    for row in 0..edge {
        for column in 0..edge {
            let index = row * edge + column;
            if column + 1 < edge {
                let right = index + 1;
                coast_edges +=
                    usize::from((sea[index] && land[right]) || (land[index] && sea[right]));
            }
            if row + 1 < edge {
                let bottom = index + edge;
                coast_edges +=
                    usize::from((sea[index] && land[bottom]) || (land[index] && sea[bottom]));
            }
            if land[index] && sea_distance[index] <= 8 {
                coastal_elevation.push(elevation[index]);
            }
            if sea[index] {
                narrow_sea += usize::from(land_distance[index] <= 8);
                if grid_neighbours(index, edge)
                    .into_iter()
                    .flatten()
                    .any(|neighbour| land[neighbour])
                {
                    sea_inlet_depth_samples = sea_inlet_depth_samples
                        .max(row.min(column).min(edge - 1 - row).min(edge - 1 - column));
                }
            }
        }
    }
    coastal_elevation.sort_by(f32::total_cmp);
    let coastal_relief_p90_metres = percentile_or_zero(&coastal_elevation, 0.90);
    let tile_side_km = edge as f32 * resolution / 1_000.0;
    let coastline_km_per_100_square_km = if tile_side_km > 0.0 {
        (coast_edges as f32 * resolution / 1_000.0) / tile_side_km.powi(2) * 100.0
    } else {
        0.0
    };
    let sea_inlet_depth_metres = sea_inlet_depth_samples as f32 * resolution;
    let narrow_sea_fraction = if sea_count == 0 {
        0.0
    } else {
        narrow_sea as f32 / sea_count as f32
    };

    let integral = integral_image(elevation, edge);
    let valley_radius = ((480.0 / resolution).round() as usize).max(1);
    let mut valley_depths = Vec::new();
    let mut deep_valleys = 0_usize;
    for row in 0..edge {
        for column in 0..edge {
            let index = row * edge + column;
            if !land[index] || sea_distance[index] <= 4 {
                continue;
            }
            let depth =
                (box_mean(&integral, edge, row, column, valley_radius) - elevation[index]).max(0.0);
            deep_valleys += usize::from(depth >= 30.0);
            valley_depths.push(depth);
        }
    }
    valley_depths.sort_by(f32::total_cmp);
    let valley_depth_p95_metres = percentile_or_zero(&valley_depths, 0.95);
    let valley_fraction = if valley_depths.is_empty() {
        0.0
    } else {
        deep_valleys as f32 / valley_depths.len() as f32
    };

    let mountain_score = (0.55 * range_score(relief_960m_p90_metres, 80.0, 450.0)
        + 0.30 * range_score(elevation_p95_metres - elevation_p50_metres, 150.0, 1_000.0)
        + 0.15 * range_score(slope_p90_degrees, 8.0, 28.0))
    .clamp(0.0, 1.0);
    let coast_balance = (4.0 * land_fraction * (1.0 - land_fraction))
        .clamp(0.0, 1.0)
        .sqrt();
    let coast_score = coast_balance
        * (0.30 * range_score(coastline_km_per_100_square_km, 4.0, 22.0)
            + 0.25 * range_score(coastal_relief_p90_metres, 40.0, 500.0)
            + 0.25 * range_score(sea_inlet_depth_metres, 300.0, 4_000.0)
            + 0.20 * range_score(narrow_sea_fraction, 0.05, 0.70));
    let valley_score = (0.65 * range_score(valley_depth_p95_metres, 12.0, 140.0)
        + 0.35 * range_score(valley_fraction, 0.02, 0.25))
    .clamp(0.0, 1.0);
    let spawn = find_spawn(
        elevation,
        &slopes,
        &sea_distance,
        edge,
        resolution,
        detail_model_origin,
    );
    let score = 100.0
        * mountain_score.max(0.01).powf(0.30)
        * coast_score.max(0.01).powf(0.30)
        * valley_score.max(0.01).powf(0.20)
        * spawn.score.max(0.01).powf(0.20);
    let center = (edge / 2) * edge + edge / 2;
    Ok(DetailWindowRank {
        latent_window,
        score,
        mountain_score,
        coast_score,
        valley_score,
        land_fraction,
        elevation_p05_metres,
        elevation_p50_metres,
        elevation_p95_metres,
        relief_960m_p90_metres,
        slope_p90_degrees,
        coastline_km_per_100_square_km,
        coastal_relief_p90_metres,
        sea_inlet_depth_metres,
        narrow_sea_fraction,
        valley_depth_p95_metres,
        valley_fraction,
        center_elevation_metres: elevation[center],
        center_slope_degrees: slopes[center],
        spawn,
        coarse_seconds,
        latent_seconds,
        decoder_seconds: detail.elapsed_seconds,
    })
}

fn find_spawn(
    elevation: &[f32],
    slopes: &[f32],
    sea_distance: &[u32],
    edge: usize,
    resolution: f32,
    detail_model_origin: [i32; 2],
) -> SpawnRank {
    let center = edge / 2;
    let search_radius = (edge / 8).max(1);
    let mut best = SpawnRank {
        score: 0.0,
        model_coordinate: [
            detail_model_origin[0] + center as i32,
            detail_model_origin[1] + center as i32,
        ],
        offset_metres: [0, 0],
        elevation_metres: elevation[center * edge + center],
        slope_degrees: slopes[center * edge + center],
        local_relief_metres: local_relief(elevation, edge, center, center, 4),
        sea_distance_metres: distance_metres(
            sea_distance[center * edge + center],
            edge,
            resolution,
        ),
    };
    for row in center.saturating_sub(search_radius)..=(center + search_radius).min(edge - 1) {
        for column in center.saturating_sub(search_radius)..=(center + search_radius).min(edge - 1)
        {
            let index = row * edge + column;
            let local_relief_metres = local_relief(elevation, edge, row, column, 4);
            let sea_distance_metres = distance_metres(sea_distance[index], edge, resolution);
            let offset_row = row as f32 - center as f32;
            let offset_column = column as f32 - center as f32;
            let center_distance =
                offset_row.hypot(offset_column) / (search_radius as f32 * 2.0_f32.sqrt());
            let score = spawn_score(
                elevation[index],
                slopes[index],
                local_relief_metres,
                sea_distance_metres,
                center_distance,
            );
            if score > best.score {
                best = SpawnRank {
                    score,
                    model_coordinate: [
                        detail_model_origin[0] + row as i32,
                        detail_model_origin[1] + column as i32,
                    ],
                    offset_metres: [
                        ((column as i32 - center as i32) as f32 * resolution).round() as i32,
                        ((row as i32 - center as i32) as f32 * resolution).round() as i32,
                    ],
                    elevation_metres: elevation[index],
                    slope_degrees: slopes[index],
                    local_relief_metres,
                    sea_distance_metres,
                };
            }
        }
    }
    best
}

fn spawn_score(
    elevation: f32,
    slope_degrees: f32,
    local_relief_metres: f32,
    sea_distance_metres: f32,
    center_distance: f32,
) -> f32 {
    if elevation <= 2.0 {
        return 0.0;
    }
    let dry = range_score(elevation, 2.0, 20.0) * (1.0 - range_score(elevation, 1_500.0, 2_500.0));
    let slope = 1.0 - range_score(slope_degrees, 4.0, 16.0);
    let relief = 1.0 - range_score(local_relief_metres, 8.0, 45.0);
    let water_safety = range_score(sea_distance_metres, 60.0, 240.0);
    let coast_access = 1.0 - 0.35 * range_score(sea_distance_metres, 3_000.0, 6_000.0);
    let centrality = 1.0 - 0.25 * center_distance.clamp(0.0, 1.0);
    (dry * slope * relief * water_safety).sqrt() * coast_access * centrality
}

fn slope_degrees(elevation: &[f32], edge: usize, resolution: f32) -> Vec<f32> {
    let mut slopes = Vec::with_capacity(elevation.len());
    for row in 0..edge {
        let top = row.saturating_sub(1);
        let bottom = (row + 1).min(edge - 1);
        let vertical_span = (bottom - top).max(1) as f32 * resolution;
        for column in 0..edge {
            let left = column.saturating_sub(1);
            let right = (column + 1).min(edge - 1);
            let horizontal_span = (right - left).max(1) as f32 * resolution;
            let horizontal =
                (elevation[row * edge + right] - elevation[row * edge + left]) / horizontal_span;
            let vertical = (elevation[bottom * edge + column] - elevation[top * edge + column])
                / vertical_span;
            slopes.push(horizontal.hypot(vertical).atan().to_degrees());
        }
    }
    slopes
}

fn boundary_connected_sea(elevation: &[f32], edge: usize) -> Vec<bool> {
    let mut sea = vec![false; elevation.len()];
    let mut queue = VecDeque::new();
    for coordinate in 0..edge {
        for index in [
            coordinate,
            (edge - 1) * edge + coordinate,
            coordinate * edge,
            coordinate * edge + edge - 1,
        ] {
            if elevation[index] <= 0.0 && !sea[index] {
                sea[index] = true;
                queue.push_back(index);
            }
        }
    }
    while let Some(index) = queue.pop_front() {
        for neighbour in grid_neighbours(index, edge).into_iter().flatten() {
            if elevation[neighbour] <= 0.0 && !sea[neighbour] {
                sea[neighbour] = true;
                queue.push_back(neighbour);
            }
        }
    }
    sea
}

fn grid_distance(sources: &[bool], edge: usize) -> Vec<u32> {
    let mut distance = vec![u32::MAX; sources.len()];
    let mut queue = VecDeque::new();
    for (index, source) in sources.iter().copied().enumerate() {
        if source {
            distance[index] = 0;
            queue.push_back(index);
        }
    }
    while let Some(index) = queue.pop_front() {
        let next_distance = distance[index].saturating_add(1);
        for neighbour in grid_neighbours(index, edge).into_iter().flatten() {
            if next_distance < distance[neighbour] {
                distance[neighbour] = next_distance;
                queue.push_back(neighbour);
            }
        }
    }
    distance
}

fn grid_neighbours(index: usize, edge: usize) -> [Option<usize>; 4] {
    let row = index / edge;
    let column = index % edge;
    [
        (row > 0).then(|| index - edge),
        (row + 1 < edge).then(|| index + edge),
        (column > 0).then(|| index - 1),
        (column + 1 < edge).then(|| index + 1),
    ]
}

fn integral_image(values: &[f32], edge: usize) -> Vec<f64> {
    let integral_edge = edge + 1;
    let mut integral = vec![0.0_f64; integral_edge * integral_edge];
    for row in 0..edge {
        let mut row_sum = 0.0_f64;
        for column in 0..edge {
            row_sum += f64::from(values[row * edge + column]);
            integral[(row + 1) * integral_edge + column + 1] =
                integral[row * integral_edge + column + 1] + row_sum;
        }
    }
    integral
}

fn box_mean(integral: &[f64], edge: usize, row: usize, column: usize, radius: usize) -> f32 {
    let first_row = row.saturating_sub(radius);
    let last_row = (row + radius + 1).min(edge);
    let first_column = column.saturating_sub(radius);
    let last_column = (column + radius + 1).min(edge);
    let integral_edge = edge + 1;
    let sum = integral[last_row * integral_edge + last_column]
        - integral[first_row * integral_edge + last_column]
        - integral[last_row * integral_edge + first_column]
        + integral[first_row * integral_edge + first_column];
    (sum / ((last_row - first_row) * (last_column - first_column)) as f64) as f32
}

fn local_relief(elevation: &[f32], edge: usize, row: usize, column: usize, radius: usize) -> f32 {
    let mut minimum = f32::INFINITY;
    let mut maximum = f32::NEG_INFINITY;
    for sample_row in row.saturating_sub(radius)..=(row + radius).min(edge - 1) {
        for sample_column in column.saturating_sub(radius)..=(column + radius).min(edge - 1) {
            let value = elevation[sample_row * edge + sample_column];
            minimum = minimum.min(value);
            maximum = maximum.max(value);
        }
    }
    maximum - minimum
}

fn distance_metres(distance_samples: u32, edge: usize, resolution: f32) -> f32 {
    distance_samples.min(edge as u32) as f32 * resolution
}

fn range_score(value: f32, minimum: f32, maximum: f32) -> f32 {
    ((value - minimum) / (maximum - minimum)).clamp(0.0, 1.0)
}

fn percentile_or_zero(sorted: &[f32], fraction: f32) -> f32 {
    if sorted.is_empty() {
        0.0
    } else {
        percentile(sorted, fraction)
    }
}

fn percentile(sorted: &[f32], fraction: f32) -> f32 {
    let position = fraction * (sorted.len() - 1) as f32;
    let lower = position.floor() as usize;
    let upper = position.ceil() as usize;
    sorted[lower] + position.fract() * (sorted[upper] - sorted[lower])
}

fn write_hillshade_preview(
    path: PathBuf,
    elevation: &[f32],
    edge: usize,
    resolution_metres: f32,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut bytes = format!("P5\n{edge} {edge}\n255\n").into_bytes();
    bytes.reserve(edge * edge);
    let light = [-0.5_f32, 2.0_f32.sqrt() / 2.0, -0.5_f32];
    for row in 0..edge {
        for column in 0..edge {
            let left = elevation[row * edge + column.saturating_sub(1)];
            let right = elevation[row * edge + (column + 1).min(edge - 1)];
            let top = elevation[row.saturating_sub(1) * edge + column];
            let bottom = elevation[(row + 1).min(edge - 1) * edge + column];
            let horizontal = (right - left) / (2.0 * resolution_metres);
            let vertical = (bottom - top) / (2.0 * resolution_metres);
            let inverse_length = (horizontal * horizontal + vertical * vertical + 1.0)
                .sqrt()
                .recip();
            let illumination = ((-horizontal * light[0] + light[1] - vertical * light[2])
                * inverse_length)
                .clamp(0.0, 1.0);
            bytes.push(((0.18 + illumination * 0.82) * 255.0).round() as u8);
        }
    }
    std::fs::write(path, bytes)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sea_mask_excludes_landlocked_water() {
        let edge = 7;
        let mut elevation = vec![10.0_f32; edge * edge];
        for row in 0..edge {
            elevation[row * edge] = -1.0;
        }
        elevation[3 * edge + 3] = -2.0;

        let sea = boundary_connected_sea(&elevation, edge);

        assert!(sea[3 * edge]);
        assert!(!sea[3 * edge + 3]);
    }

    #[test]
    fn spawn_rank_prefers_the_flat_safe_center() {
        let edge = 64;
        let elevation = vec![80.0_f32; edge * edge];
        let slopes = vec![0.0_f32; edge * edge];
        let mut sea = vec![false; edge * edge];
        for row in 0..edge {
            for column in 0..8 {
                sea[row * edge + column] = true;
            }
        }

        let spawn = find_spawn(
            &elevation,
            &slopes,
            &grid_distance(&sea, edge),
            edge,
            30.0,
            [100, 200],
        );

        assert_eq!(spawn.model_coordinate, [132, 232]);
        assert_eq!(spawn.offset_metres, [0, 0]);
        assert!(spawn.score > 0.95);
        assert!(spawn.sea_distance_metres >= 700.0);
    }

    #[test]
    fn spawn_offsets_are_reported_in_world_xz_order() {
        let edge = 64;
        let center = edge / 2;
        let target_row = center + 4;
        let target_column = center + 7;
        let mut elevation = vec![-20.0_f32; edge * edge];
        let slopes = vec![0.0_f32; edge * edge];
        let mut sea = vec![true; edge * edge];
        for row in target_row - 4..=target_row + 4 {
            for column in target_column - 4..=target_column + 4 {
                elevation[row * edge + column] = 80.0;
                sea[row * edge + column] = false;
            }
        }

        let spawn = find_spawn(
            &elevation,
            &slopes,
            &grid_distance(&sea, edge),
            edge,
            30.0,
            [100, 200],
        );

        assert_eq!(
            spawn.model_coordinate,
            [100 + target_row as i32, 200 + target_column as i32]
        );
        assert_eq!(spawn.offset_metres, [210, 120]);
    }

    #[test]
    fn detail_ranker_measures_boundary_connected_fjord() {
        let edge = 64;
        let mut elevation = vec![60.0_f32; edge * edge];
        for row in 0..edge {
            for column in 0..12 {
                elevation[row * edge + column] = -20.0;
            }
        }
        for row in 29..35 {
            for column in 12..44 {
                elevation[row * edge + column] = -5.0;
            }
        }
        for row in 0..edge {
            for column in 44..edge {
                elevation[row * edge + column] += (column - 43) as f32 * 18.0;
            }
        }
        let detail = DetailTile {
            edge,
            horizontal_resolution_metres: 30,
            elevation_metres: elevation,
            residual_sqrt_elevation: vec![0.0; edge * edge],
            elapsed_seconds: 0.25,
        };

        let rank = rank_detail_window([-2, -2], [-512, -512], &detail, 0.5, 0.4)
            .expect("synthetic detail should rank");

        assert!(rank.coastline_km_per_100_square_km > 5.0);
        assert!(rank.sea_inlet_depth_metres >= 570.0);
        assert!(rank.coast_score > 0.1);
        assert!(rank.mountain_score > 0.1);
        assert!(rank.spawn.score > 0.5);
    }
}
