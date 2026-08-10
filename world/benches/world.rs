#![allow(
    clippy::expect_used,
    clippy::panic,
    reason = "invalid deterministic benchmark fixtures must fail instead of silently skipping samples"
)]

use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use std::hint::black_box;
use voxels_world::codec::{decode_chunk, encode_chunk};
use voxels_world::protocol::{
    ChunkBatchItem, ChunkBatchResult, clone_message_with_request_id, decode_chunk_batch_result,
    encode_chunk_batch_result,
};
use voxels_world::{
    BinaryMeshScratch, CHUNK_EDGE, CINDER_VAULT, ChunkCoord, EditMap, Generator, Material,
    MeshingHalo, ProceduralWorldSource, SkylineFeatureKind, TERRAIN_COVERAGE_ROOT_LEVEL,
    TerrainPageKey, VoxelCoord, WorldProduct, WorldProductBatch, WorldProductPriority,
    WorldProductRequest, WorldSourceEngine, first_pilgrim_road_length_voxels,
    first_pilgrim_road_point_at_distance, first_pilgrim_route_anchor,
    first_pilgrim_route_anchor_for_feature_cell, mesh_chunk_binary_with_scratch,
    sample_first_pilgrim_road,
};

const SEED: u64 = 0x5eed_cafe;
const COORD: ChunkCoord = ChunkCoord::new(2, 0, -3);
const OCEAN_VOXEL: VoxelCoord = VoxelCoord::new(18_016, 10, 12_896);

fn generation(criterion: &mut Criterion) {
    let generator = Generator::new(SEED);
    criterion.bench_function("generate 32^3 chunk", |bencher| {
        bencher.iter(|| generator.generate_chunk(COORD));
    });

    let road_length = first_pilgrim_road_length_voxels();
    let (road, _) = first_pilgrim_road_point_at_distance(road_length * 0.5)
        .expect("the fixed pilgrim-road benchmark distance must remain valid");
    let road_x = road[0].round() as i32;
    let road_z = road[1].round() as i32;
    let road_y = generator.surface_height(road_x, road_z);
    let road_coord = VoxelCoord::new(road_x, road_y, road_z).chunk();
    criterion.bench_function("generate 32^3 pilgrim-road chunk", |bencher| {
        bencher.iter(|| generator.generate_chunk(road_coord));
    });

    let chamber = CINDER_VAULT.chamber;
    let cave_coord = VoxelCoord::new(chamber[0], chamber[1], chamber[2]).chunk();
    criterion.bench_function("generate 32^3 Cinder Vault chunk", |bencher| {
        bencher.iter(|| generator.generate_chunk(cave_coord));
    });
}

fn source_products(criterion: &mut Criterion) {
    let source = ProceduralWorldSource::new(SEED);
    criterion.bench_function("generate one chunk + 6,536-cell meshing halo", |bencher| {
        bencher.iter(|| {
            source.generate_batch(WorldProductBatch {
                priority: WorldProductPriority::VisibleChunk,
                requests: vec![WorldProductRequest::ChunkWithHalo(COORD)],
            })
        });
    });

    criterion.bench_function(
        "generate two chunk + halo products as one batch",
        |bencher| {
            bencher.iter(|| {
                source.generate_batch(WorldProductBatch {
                    priority: WorldProductPriority::VisibleChunk,
                    requests: vec![
                        WorldProductRequest::ChunkWithHalo(COORD),
                        WorldProductRequest::ChunkWithHalo(ChunkCoord::new(3, 0, -3)),
                    ],
                })
            });
        },
    );

    let origin = COORD.world_origin();
    let region = Generator::new(SEED).region(origin[0] - 1, origin[2] - 1, 34, 34);
    criterion.bench_function("materialize 6,536-cell meshing halo", |bencher| {
        bencher.iter(|| MeshingHalo::from_sampler(COORD, |x, y, z| region.sample(x, y, z)));
    });
}

fn semantic_hero_generation(criterion: &mut Criterion) {
    let generator = Generator::new(SEED);
    let hero = generator
        .nearest_prominent_skyline_feature(0, 0, SkylineFeatureKind::ElderCanopy, 192)
        .expect("the fixed benchmark seed must retain its elder-canopy fixture");
    let hero_chunk = VoxelCoord::new(hero.anchor[0], hero.trunk_top, hero.anchor[2]).chunk();
    criterion.bench_function("generate 32^3 elder-canopy hero chunk", |bencher| {
        bencher.iter(|| generator.generate_chunk(hero_chunk));
    });
}

fn route_queries(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("pilgrim-road indexed queries");
    group.bench_function("global bounds reject", |bencher| {
        bencher.iter(|| {
            let (x, z) = black_box((10_000, -10_000));
            sample_first_pilgrim_road(x, z)
        });
    });
    group.bench_function("segment corridor reject", |bencher| {
        bencher.iter(|| {
            let (x, z) = black_box((-1_200, 0));
            sample_first_pilgrim_road(x, z)
        });
    });
    group.bench_function("near-segment projection", |bencher| {
        bencher.iter(|| {
            let (x, z) = black_box((-632, 656));
            sample_first_pilgrim_road(x, z)
        });
    });
    let distance = first_pilgrim_road_length_voxels() * 0.73;
    group.bench_function("point at cumulative distance", |bencher| {
        bencher.iter(|| first_pilgrim_road_point_at_distance(black_box(distance)));
    });
    if let Some(anchor) = first_pilgrim_route_anchor(3) {
        group.bench_function("station feature-cell lookup", |bencher| {
            bencher.iter(|| {
                let cell = black_box(anchor.feature_cell);
                first_pilgrim_route_anchor_for_feature_cell(cell[0], cell[1])
            });
        });
    }
    group.finish();
}

fn edit_column_snapshots(criterion: &mut Criterion) {
    let root = TerrainPageKey::surface(TERRAIN_COVERAGE_ROOT_LEVEL, 0, 0);
    let [[minimum_x, minimum_z], [maximum_x, maximum_z]] = root
        .horizontal_bounds()
        .expect("fixed level-10 surface root must have finite bounds");
    let empty = EditMap::default();

    let mut sparse = EditMap::default();
    for index in 0..64 {
        sparse.insert_override(
            VoxelCoord::new(index * 32, index * 32, index * 32),
            Material::Stone,
        );
    }
    for chunk_x in 0..64 {
        for chunk_z in 2_048..2_112 {
            sparse.insert_override(
                VoxelCoord::new(chunk_x * 32, 0, chunk_z * 32),
                Material::Basalt,
            );
        }
    }

    let mut dense = EditMap::default();
    for chunk_x in -32..32 {
        for chunk_z in -32..32 {
            dense.insert_override(
                VoxelCoord::new(chunk_x * 32, 0, chunk_z * 32),
                Material::Clay,
            );
        }
    }
    dense.insert_override(VoxelCoord::new(0, -64, 0), Material::Stone);
    dense.insert_override(VoxelCoord::new(0, 64, 0), Material::Wood);

    let mut group = criterion.benchmark_group("edit column snapshot");
    group.bench_function("level-10 empty", |bencher| {
        bencher.iter(|| {
            black_box(black_box(&empty).snapshot_for_voxel_columns(
                black_box(minimum_x),
                black_box(maximum_x),
                black_box(minimum_z),
                black_box(maximum_z),
            ))
        });
    });
    group.bench_function("level-10 sparse root plus far edits", |bencher| {
        bencher.iter(|| {
            black_box(black_box(&sparse).snapshot_for_voxel_columns(
                black_box(minimum_x),
                black_box(maximum_x),
                black_box(minimum_z),
                black_box(maximum_z),
            ))
        });
    });
    group.bench_function("one column in dense journal", |bencher| {
        bencher.iter(|| {
            black_box(black_box(&dense).snapshot_for_voxel_columns(
                black_box(0),
                black_box(0),
                black_box(0),
                black_box(0),
            ))
        });
    });
    group.finish();
}

fn codec(criterion: &mut Criterion) {
    let source = ProceduralWorldSource::new(SEED);
    let identity = source.source_identity_hash();
    let chunk = Generator::new(SEED).generate_chunk(COORD);
    let encoded = encode_chunk(&chunk, identity);
    let mut group = criterion.benchmark_group("VXCH palette codec");
    group.throughput(criterion::Throughput::Bytes(
        (chunk.voxels().len() * size_of::<u16>()) as u64,
    ));
    group.bench_function("encode", |bencher| {
        bencher.iter(|| encode_chunk(&chunk, identity));
    });
    group.bench_function("decode", |bencher| {
        bencher.iter(|| decode_chunk(&encoded, identity));
    });
    group.finish();

    let batch = source
        .generate_batch(WorldProductBatch {
            priority: WorldProductPriority::VisibleChunk,
            requests: vec![WorldProductRequest::ChunkWithHalo(COORD)],
        })
        .expect("fixed chunk benchmark product must generate");
    let item = batch
        .items
        .into_iter()
        .next()
        .expect("fixed chunk benchmark batch must contain its request");
    let snapshot = match item.result {
        Ok(WorldProduct::Chunk(snapshot)) => snapshot,
        result => panic!("fixed chunk benchmark returned {result:?}"),
    };
    let response = ChunkBatchResult {
        request_id: 1,
        source_identity_hash: identity,
        items: vec![ChunkBatchItem {
            coord: COORD,
            edit_revision: 1,
            result: Ok(snapshot),
        }],
    };
    let wire =
        encode_chunk_batch_result(&response).expect("fixed chunk benchmark response must encode");
    let mut group = criterion.benchmark_group("VXWP chunk + halo envelope");
    group.throughput(criterion::Throughput::Bytes(wire.len() as u64));
    group.bench_function("encode", |bencher| {
        bencher.iter(|| encode_chunk_batch_result(&response));
    });
    group.bench_function("decode", |bencher| {
        bencher.iter(|| decode_chunk_batch_result(&wire));
    });
    group.finish();
}

fn streaming_codec(criterion: &mut Criterion) {
    let source = ProceduralWorldSource::new(SEED);
    let identity = source.source_identity_hash();
    let generator = Generator::new(SEED);
    let chunk_coords = (-1..=1)
        .flat_map(|z| {
            let generator = &generator;
            (-1..=1).map(move |x| {
                let edge = CHUNK_EDGE as i32;
                let surface = generator.surface_height(x * edge + edge / 2, z * edge + edge / 2);
                ChunkCoord::new(x, surface.div_euclid(edge), z)
            })
        })
        .collect::<Vec<_>>();
    let chunk_products = source
        .generate_batch(WorldProductBatch {
            priority: WorldProductPriority::VisibleChunk,
            requests: chunk_coords
                .iter()
                .copied()
                .map(WorldProductRequest::ChunkWithHalo)
                .collect(),
        })
        .expect("fixed 3x3 chunk stream must generate");
    let chunk_response = ChunkBatchResult {
        request_id: 2,
        source_identity_hash: identity,
        items: chunk_products
            .items
            .into_iter()
            .filter_map(|item| match (item.request, item.result) {
                (WorldProductRequest::ChunkWithHalo(coord), Ok(WorldProduct::Chunk(snapshot))) => {
                    Some(ChunkBatchItem {
                        coord,
                        edit_revision: 1,
                        result: Ok(snapshot),
                    })
                }
                _ => None,
            })
            .collect(),
    };
    assert_eq!(chunk_response.items.len(), chunk_coords.len());
    let chunk_wire =
        encode_chunk_batch_result(&chunk_response).expect("fixed 3x3 chunk stream must encode");
    let mut group = criterion.benchmark_group(format!(
        "VXWP 3x3 chunk stream ({} wire bytes)",
        chunk_wire.len()
    ));
    group.throughput(criterion::Throughput::Bytes(chunk_wire.len() as u64));
    group.bench_function("encode", |bencher| {
        bencher.iter(|| encode_chunk_batch_result(&chunk_response));
    });
    group.bench_function("decode", |bencher| {
        bencher.iter(|| decode_chunk_batch_result(&chunk_wire));
    });
    group.bench_function("cached frame clone", |bencher| {
        bencher.iter(|| clone_message_with_request_id(&chunk_wire, 4));
    });
    group.finish();
}

fn meshing(criterion: &mut Criterion) {
    let generator = Generator::new(SEED);
    let mut scratch = BinaryMeshScratch::default();
    criterion.bench_function("binary mesh generated chunk", |bencher| {
        bencher.iter_batched(
            || {
                (
                    generator.generate_chunk(COORD),
                    MeshingHalo::from_sampler(COORD, |x, y, z| generator.sample(x, y, z)),
                )
            },
            |(chunk, halo)| {
                mesh_chunk_binary_with_scratch(
                    &chunk,
                    |x, y, z| {
                        halo.sample_world(x, y, z)
                            .expect("benchmark halo must cover the binary mesher shell")
                    },
                    &mut scratch,
                )
            },
            BatchSize::SmallInput,
        );
    });
    let chamber = CINDER_VAULT.chamber;
    let cave_coord = VoxelCoord::new(chamber[0], chamber[1], chamber[2]).chunk();
    criterion.bench_function("binary mesh Cinder Vault chunk", |bencher| {
        bencher.iter_batched(
            || {
                (
                    generator.generate_chunk(cave_coord),
                    MeshingHalo::from_sampler(cave_coord, |x, y, z| generator.sample(x, y, z)),
                )
            },
            |(chunk, halo)| {
                mesh_chunk_binary_with_scratch(
                    &chunk,
                    |x, y, z| {
                        halo.sample_world(x, y, z)
                            .expect("benchmark halo must cover the binary mesher shell")
                    },
                    &mut scratch,
                )
            },
            BatchSize::SmallInput,
        );
    });
}

fn water_meshing(criterion: &mut Criterion) {
    let generator = Generator::new(SEED);
    let coord = OCEAN_VOXEL.chunk();
    let mut scratch = BinaryMeshScratch::default();
    criterion.bench_function("binary mesh generated ocean chunk", |bencher| {
        bencher.iter_batched(
            || {
                (
                    generator.generate_chunk(coord),
                    MeshingHalo::from_sampler(coord, |x, y, z| generator.sample(x, y, z)),
                )
            },
            |(chunk, halo)| {
                mesh_chunk_binary_with_scratch(
                    &chunk,
                    |x, y, z| {
                        halo.sample_world(x, y, z)
                            .expect("benchmark halo must cover the binary mesher shell")
                    },
                    &mut scratch,
                )
            },
            BatchSize::SmallInput,
        );
    });
}

criterion_group!(
    world_benches,
    generation,
    source_products,
    semantic_hero_generation,
    route_queries,
    edit_column_snapshots,
    codec,
    streaming_codec,
    meshing,
    water_meshing,
);
criterion_main!(world_benches);
