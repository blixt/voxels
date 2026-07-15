#[cfg(not(target_os = "macos"))]
compile_error!("voxels-terrain-diffusion requires macOS and Apple Metal");

use std::path::PathBuf;
use voxels_world_terrain_diffusion::{
    MetalTerrainDiffusion, TerrainDiffusionConfig, TerrainPrecision, fetch_pinned_model,
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
        && command != "survey-smoke"
        && command != "counterproof"
    {
        return Err(format!(
            "unknown command {command}; expected fetch, smoke, base-smoke, detail-smoke, survey-smoke, or counterproof"
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
        .unwrap_or([0, 0]);
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
    let runtime = if command == "detail-smoke" || command == "counterproof" {
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
        return Ok(());
    }
    if command == "survey-smoke" {
        let radius = std::env::var("VOXELS_TERRAIN_SURVEY_RADIUS")
            .ok()
            .map(|value| value.parse::<i32>())
            .transpose()?
            .unwrap_or(2);
        if !(0..=8).contains(&radius) {
            return Err("VOXELS_TERRAIN_SURVEY_RADIUS must be in 0..=8".into());
        }
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
                candidates.push((
                    variance.sqrt(),
                    [row, column],
                    minimum,
                    maximum,
                    coarse.elapsed_seconds,
                    latent.elapsed_seconds,
                ));
            }
        }
        candidates.sort_by(|left, right| right.0.total_cmp(&left.0));
        println!("device={}", runtime.device_description());
        println!("precision={:?}", runtime.precision());
        println!("seed={seed} center={latent_window:?} radius={radius}");
        for (rank, (standard_deviation, window, minimum, maximum, coarse_s, latent_s)) in
            candidates.into_iter().enumerate()
        {
            println!(
                "rank={} latent_window={window:?} low_std={standard_deviation:.6} low_min={minimum:.6} low_max={maximum:.6} coarse_ms={:.3} latent_ms={:.3}",
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
