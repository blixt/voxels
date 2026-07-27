#[cfg(not(target_os = "macos"))]
compile_error!("voxels-virtual-surface-bakeoff requires macOS and Apple Metal");

use serde_json::{Value, json};
use std::path::PathBuf;
use std::time::Instant;
use voxels_world::{
    BakeoffCamera, BakeoffCandidateKind, BakeoffVolume, Material, SurfaceSampleBlockRequest,
    VoxelBlockRequest, VoxelBounds, VoxelCoord, WorldProduct, WorldProductBatch,
    WorldProductPriority, WorldProductRequest, WorldSourceEngine, run_virtual_surface_bakeoff,
};
use voxels_world_service::LoadedWorldServiceConfig;

mod gpu;
mod gpu_voxel;

const SUPPLIED_SOURCE_HASH: &str =
    "82bdc2f68c8aa5a845927e52c2e3c5c781e96a7fe83b1bc723384df91daae09f";
const DEFAULT_EDGE: u32 = 128;
const BLOCK_EDGE: u32 = 32;
const BELOW_SURFACE_VOXELS: i32 = 64;
const ABOVE_SURFACE_VOXELS: i32 = 32;

#[derive(Clone, Copy)]
struct SuppliedPose {
    eye_metres: [f64; 3],
    yaw: f64,
    pitch: f64,
    pixel_size: [u32; 2],
}

const SUPPLIED_POSES: [SuppliedPose; 3] = [
    SuppliedPose {
        eye_metres: [1961.5779, 54.665_314, -1616.0098],
        yaw: 2.607_078_8,
        pitch: -0.598_799_94,
        pixel_size: [3840, 1814],
    },
    SuppliedPose {
        eye_metres: [1966.626, 54.665_314, -1633.4033],
        yaw: 2.609_279,
        pitch: -0.596_599_94,
        pixel_size: [3840, 1814],
    },
    SuppliedPose {
        eye_metres: [1949.6088, 58.603_07, -1651.7283],
        yaw: 2.750_077_5,
        pitch: -0.497_600_2,
        pixel_size: [3840, 1814],
    },
];

struct Arguments {
    config: PathBuf,
    output: Option<PathBuf>,
    fixture: Fixture,
    gpu: bool,
    pose: usize,
    edge: u32,
    ray_grid: [u32; 2],
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum Fixture {
    Supplied,
    TopologyStress,
}

fn parse_arguments() -> Result<Arguments, Box<dyn std::error::Error>> {
    let mut config = PathBuf::from("config/world-service.toml");
    let mut output = None;
    let mut fixture = Fixture::Supplied;
    let mut gpu = false;
    let mut pose = 0usize;
    let mut edge = DEFAULT_EDGE;
    let mut ray_grid = [320, 180];
    for argument in std::env::args().skip(1) {
        if let Some(value) = argument.strip_prefix("--config=") {
            config = PathBuf::from(value);
        } else if let Some(value) = argument.strip_prefix("--output=") {
            output = Some(PathBuf::from(value));
        } else if let Some(value) = argument.strip_prefix("--fixture=") {
            fixture = match value {
                "supplied" => Fixture::Supplied,
                "topology-stress" => Fixture::TopologyStress,
                _ => return Err("--fixture must be supplied or topology-stress".into()),
            };
        } else if argument == "--gpu" {
            gpu = true;
        } else if let Some(value) = argument.strip_prefix("--pose=") {
            pose = value.parse::<usize>()?.saturating_sub(1);
        } else if let Some(value) = argument.strip_prefix("--edge=") {
            edge = value.parse()?;
        } else if let Some(value) = argument.strip_prefix("--rays=") {
            let (width, height) = value
                .split_once('x')
                .ok_or("ray grid must be formatted WIDTHxHEIGHT")?;
            ray_grid = [width.parse()?, height.parse()?];
        } else {
            return Err(format!("unknown argument {argument}").into());
        }
    }
    if pose >= SUPPLIED_POSES.len() {
        return Err("--pose must be 1, 2, or 3".into());
    }
    if edge == 0 || edge > 512 {
        return Err("--edge must be in 1..=512".into());
    }
    Ok(Arguments {
        config,
        output,
        fixture,
        gpu,
        pose,
        edge,
        ray_grid,
    })
}

fn topology_stress_volume() -> Result<(BakeoffVolume, Value), Box<dyn std::error::Error>> {
    let bounds = VoxelBounds::new(VoxelCoord::new(-64, -32, -64), VoxelCoord::new(64, 48, 64))
        .ok_or("topology-stress bounds are invalid")?;
    let volume = BakeoffVolume::from_sampler(bounds, |coord| {
        let tunnel = (-7..=7).contains(&coord.x)
            && (-48..=16).contains(&coord.z)
            && (-9..=0).contains(&coord.y);
        let cave = {
            let dx = i64::from(coord.x + 28);
            let dy = i64::from(coord.y + 10);
            let dz = i64::from(coord.z + 20);
            dx * dx + dy * dy + dz * dz < 14 * 14
        };
        let floating_voxel = coord == VoxelCoord::new(24, 22, -12);
        let overhang = (-48..=-12).contains(&coord.x)
            && (8..=11).contains(&coord.y)
            && (-30..=12).contains(&coord.z);
        let overhang_support = (-48..=-43).contains(&coord.x)
            && (0..=11).contains(&coord.y)
            && (-30..=12).contains(&coord.z);
        let water = (12..=42).contains(&coord.x)
            && (-30..=8).contains(&coord.z)
            && (1..=3).contains(&coord.y);
        if floating_voxel {
            Material::GlowCrystal
        } else if overhang || overhang_support {
            Material::Basalt
        } else if water {
            Material::Water
        } else if coord.y <= 0 && !tunnel && !cave {
            if coord.y == 0 {
                Material::Grass
            } else if coord.y >= -3 {
                Material::Dirt
            } else {
                Material::Stone
            }
        } else {
            Material::Air
        }
    })?;
    Ok((
        volume,
        json!({
            "bounds": {
                "min": bounds.min.as_array(),
                "max": bounds.max.as_array(),
                "shape": [128, 80, 128],
            },
            "features": [
                "tunnel-roof",
                "enclosed-cave",
                "supported-overhang",
                "floating-single-voxel",
                "opaque-material-runs",
                "water-terrain-intersection",
            ],
        }),
    ))
}

fn single_product(
    source: &dyn WorldSourceEngine,
    request: WorldProductRequest,
) -> Result<WorldProduct, Box<dyn std::error::Error>> {
    let mut result = source.generate_batch(WorldProductBatch {
        priority: WorldProductPriority::VisibleSurface,
        requests: vec![request],
    })?;
    if result.items.len() != 1 {
        return Err("world source returned the wrong item count".into());
    }
    result
        .items
        .pop()
        .ok_or_else(|| "world source omitted the requested product".into())
        .and_then(|item| item.result.map_err(Into::into))
}

fn sample_volume(
    source: &dyn WorldSourceEngine,
    pose: SuppliedPose,
    edge: u32,
) -> Result<(BakeoffVolume, Value), Box<dyn std::error::Error>> {
    let eye_voxels = pose.eye_metres.map(|metres| metres * 10.0);
    let forward = [
        pose.yaw.sin() * pose.pitch.cos(),
        pose.pitch.sin(),
        -pose.yaw.cos() * pose.pitch.cos(),
    ];
    let focus = find_surface_focus(source, eye_voxels, forward)?;
    let half = i32::try_from(edge / 2)?;
    let min_x = focus[0].saturating_sub(half);
    let min_z = focus[1].saturating_sub(half);
    let surface_request = SurfaceSampleBlockRequest {
        origin: [min_x, min_z],
        sample_shape: [edge, edge],
    };
    let WorldProduct::SurfaceSampleBlock(surface) = single_product(
        source,
        WorldProductRequest::SurfaceSampleBlock(surface_request),
    )?
    else {
        return Err("world source returned the wrong surface product".into());
    };
    let minimum_height = surface
        .samples()
        .iter()
        .map(|sample| sample.height)
        .min()
        .ok_or("surface product is empty")?;
    let maximum_height = surface
        .samples()
        .iter()
        .map(|sample| sample.height.max(sample.water_level.unwrap_or(i32::MIN)))
        .max()
        .ok_or("surface product is empty")?;
    let min_y = minimum_height.saturating_sub(BELOW_SURFACE_VOXELS);
    let max_y = maximum_height
        .saturating_add(ABOVE_SURFACE_VOXELS)
        .saturating_add(1);
    let height = u32::try_from(i64::from(max_y) - i64::from(min_y))?;
    let bounds = VoxelBounds::new(
        VoxelCoord::new(min_x, min_y, min_z),
        VoxelCoord::new(
            min_x.saturating_add(i32::try_from(edge)?),
            max_y,
            min_z.saturating_add(i32::try_from(edge)?),
        ),
    )
    .ok_or("computed volume bounds are invalid")?;
    let sample_count = usize::try_from(edge)?
        .checked_mul(usize::try_from(height)?)
        .and_then(|plane| plane.checked_mul(usize::try_from(edge).ok()?))
        .ok_or("volume sample count overflows")?;
    let mut materials = vec![Material::Air; sample_count];
    let mut requests = Vec::new();
    let mut z = 0u32;
    while z < edge {
        let depth = BLOCK_EDGE.min(edge - z);
        let mut x = 0u32;
        while x < edge {
            let width = BLOCK_EDGE.min(edge - x);
            requests.push(WorldProductRequest::VoxelBlock(VoxelBlockRequest {
                min: VoxelCoord::new(
                    min_x.saturating_add(i32::try_from(x)?),
                    min_y,
                    min_z.saturating_add(i32::try_from(z)?),
                ),
                sample_shape: [width, height, depth],
            }));
            x += width;
        }
        z += depth;
    }
    if requests.len() > 256 {
        return Err("volume requires more than one bounded world-product batch".into());
    }
    let result = source.generate_batch(WorldProductBatch {
        priority: WorldProductPriority::VisibleChunk,
        requests,
    })?;
    for item in result.items {
        let WorldProduct::VoxelBlock(block) = item.result? else {
            return Err("world source returned the wrong voxel product".into());
        };
        let [width, block_height, depth] = block.request.sample_shape;
        for local_z in 0..depth {
            for local_y in 0..block_height {
                for local_x in 0..width {
                    let world = VoxelCoord::new(
                        block.request.min.x.saturating_add(i32::try_from(local_x)?),
                        block.request.min.y.saturating_add(i32::try_from(local_y)?),
                        block.request.min.z.saturating_add(i32::try_from(local_z)?),
                    );
                    let material = block
                        .sample(world)
                        .ok_or("voxel product omitted an in-bounds sample")?;
                    let global_x = usize::try_from(i64::from(world.x) - i64::from(bounds.min.x))?;
                    let global_y = usize::try_from(i64::from(world.y) - i64::from(bounds.min.y))?;
                    let global_z = usize::try_from(i64::from(world.z) - i64::from(bounds.min.z))?;
                    let index = global_x
                        + global_y * usize::try_from(edge)?
                        + global_z * usize::try_from(edge)? * usize::try_from(height)?;
                    materials[index] = material;
                }
            }
        }
    }
    let volume = BakeoffVolume::new(bounds, materials)?;
    Ok((
        volume,
        json!({
            "bounds": {
                "min": bounds.min.as_array(),
                "max": bounds.max.as_array(),
                "shape": [edge, height, edge],
            },
            "surfaceHeightVoxels": {
                "minimum": minimum_height,
                "maximum": maximum_height,
            },
            "lookIntersectionVoxels": focus,
        }),
    ))
}

fn find_surface_focus(
    source: &dyn WorldSourceEngine,
    eye: [f64; 3],
    forward: [f64; 3],
) -> Result<[i32; 2], Box<dyn std::error::Error>> {
    const STEP_VOXELS: u32 = 8;
    const SAMPLE_COUNT: u32 = 256;
    let requests = (0..SAMPLE_COUNT)
        .map(|index| {
            let distance = f64::from(index * STEP_VOXELS);
            WorldProductRequest::SurfaceSampleBlock(SurfaceSampleBlockRequest {
                origin: [
                    (eye[0] + forward[0] * distance).floor() as i32,
                    (eye[2] + forward[2] * distance).floor() as i32,
                ],
                sample_shape: [1, 1],
            })
        })
        .collect::<Vec<_>>();
    let result = source.generate_batch(WorldProductBatch {
        priority: WorldProductPriority::VisibleSurface,
        requests,
    })?;
    for (index, item) in result.items.into_iter().enumerate() {
        let WorldProduct::SurfaceSampleBlock(block) = item.result? else {
            return Err("world source returned the wrong focus product".into());
        };
        let sample = block
            .samples()
            .first()
            .ok_or("world source returned an empty focus sample")?;
        let distance = f64::from(u32::try_from(index)? * STEP_VOXELS);
        let ray_y = eye[1] + forward[1] * distance;
        let surface_y = f64::from(sample.height.max(sample.water_level.unwrap_or(i32::MIN)) + 1);
        if ray_y <= surface_y {
            return Ok(block.request.origin);
        }
    }
    Err("camera centre ray did not reach authoritative terrain within 204.8 metres".into())
}

#[allow(
    clippy::print_stdout,
    reason = "the explicit bake-off command emits its machine-readable report"
)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arguments = parse_arguments()?;
    let volume_started = Instant::now();
    let (volume, region, camera, source_json, capture_json) = match arguments.fixture {
        Fixture::Supplied => {
            let loaded = LoadedWorldServiceConfig::load(&arguments.config)?;
            let source_started = Instant::now();
            let source = loaded.build_world_source()?;
            let source_build_ms = source_started.elapsed().as_secs_f64() * 1000.0;
            let identity_hash = source.identity().identity_hash().to_string();
            if identity_hash != SUPPLIED_SOURCE_HASH {
                return Err(format!(
                    "configured source {identity_hash} does not match supplied captures {SUPPLIED_SOURCE_HASH}"
                )
                .into());
            }
            let pose = SUPPLIED_POSES[arguments.pose];
            let (volume, region) = sample_volume(source.as_ref(), pose, arguments.edge)?;
            let camera = BakeoffCamera {
                eye_voxels: pose.eye_metres.map(|metres| metres * 10.0),
                yaw_radians: pose.yaw,
                pitch_radians: pose.pitch,
                vertical_fov_radians: 1.186_823_8,
                aspect_ratio: f64::from(pose.pixel_size[0]) / f64::from(pose.pixel_size[1]),
            };
            (
                volume,
                region,
                camera,
                json!({
                    "config": arguments.config,
                    "identityHash": identity_hash,
                    "seed": loaded.config().world_seed.to_string(),
                    "buildMs": source_build_ms,
                }),
                json!({
                    "fixture": "supplied",
                    "pose": arguments.pose + 1,
                    "eyeMetres": pose.eye_metres,
                    "yawRadians": pose.yaw,
                    "pitchRadians": pose.pitch,
                    "verticalFovRadians": 1.186_823_8,
                    "pixelSize": pose.pixel_size,
                    "rayGrid": arguments.ray_grid,
                }),
            )
        }
        Fixture::TopologyStress => {
            let (volume, region) = topology_stress_volume()?;
            (
                volume,
                region,
                BakeoffCamera {
                    eye_voxels: [0.5, 28.5, 76.0],
                    yaw_radians: 0.0,
                    pitch_radians: -0.28,
                    vertical_fov_radians: 1.0,
                    aspect_ratio: 16.0 / 9.0,
                },
                json!({
                    "identityHash": "deterministic-topology-stress-v1",
                    "seed": "0",
                    "buildMs": 0,
                }),
                json!({
                    "fixture": "topology-stress",
                    "pose": 0,
                    "eyeVoxels": [0.5, 28.5, 76.0],
                    "yawRadians": 0,
                    "pitchRadians": -0.28,
                    "verticalFovRadians": 1,
                    "pixelSize": [16, 9],
                    "rayGrid": arguments.ray_grid,
                }),
            )
        }
    };
    let volume_sample_ms = volume_started.elapsed().as_secs_f64() * 1000.0;
    let (candidates, comparisons) = run_virtual_surface_bakeoff(
        &volume,
        camera,
        arguments.ray_grid,
        &BakeoffCandidateKind::ALL,
    )?;
    let gpu_report = if arguments.gpu {
        let clustered = candidates
            .iter()
            .find(|candidate| candidate.kind == BakeoffCandidateKind::ClusteredVirtualGeometry)
            .ok_or("bake-off omitted clustered candidate")?;
        let quads = clustered
            .gpu_quads()
            .ok_or("clustered candidate omitted exact GPU leaves")?;
        let exact_cluster_raster = pollster::block_on(gpu::run(camera, &quads))?;
        let dense_voxel_ray_caster = pollster::block_on(gpu_voxel::run(camera, &volume))?;
        json!({
            "schema": "voxels.virtual-surface-gpu-competition.v1",
            "exactClusterRaster": exact_cluster_raster,
            "denseVoxelRayCasterLowerBound": dense_voxel_ray_caster,
        })
    } else {
        Value::Null
    };
    let candidate_json = candidates
        .iter()
        .zip(comparisons.iter())
        .map(|(candidate, comparison)| {
            json!({
                "kind": candidate.kind.label(),
                "buildMs": candidate.build_time.as_secs_f64() * 1000.0,
                "traceMs": comparison.trace_time.as_secs_f64() * 1000.0,
                "logicalBytes": candidate.logical_bytes,
                "primitiveCount": candidate.primitive_count,
                "volumetricExceptionColumns": candidate.volumetric_exception_columns,
                "rays": comparison.rays,
                "referenceHits": comparison.reference_hits,
                "candidateHits": comparison.candidate_hits,
                "ownerlessReferenceHits": comparison.ownerless_reference_hits,
                "inventedHits": comparison.invented_hits,
                "materialMismatches": comparison.material_mismatches,
                "depthMismatches": comparison.depth_mismatches,
                "maximumDepthErrorVoxels": comparison.maximum_depth_error_voxels,
            })
        })
        .collect::<Vec<_>>();
    let report = json!({
        "schema": "voxels.virtual-surface-bakeoff.v1",
        "source": source_json,
        "capture": capture_json,
        "region": region,
        "volume": {
            "logicalBytes": volume.logical_bytes(),
            "sampleMs": volume_sample_ms,
        },
        "candidates": candidate_json,
        "gpu": gpu_report,
    });
    let encoded = serde_json::to_string_pretty(&report)?;
    if let Some(path) = arguments.output {
        std::fs::write(path, format!("{encoded}\n"))?;
    } else {
        println!("{encoded}");
    }
    Ok(())
}
