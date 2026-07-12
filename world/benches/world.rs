use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use std::hint::black_box;
use voxels_world::codec::{decode_chunk, encode_chunk};
use voxels_world::{
    ChunkCoord, EditMap, FarTileCoord, Generator, Material, SkylineFeatureKind, SurfaceLodLevel,
    SurfaceTileCoord, VoxelCoord, first_pilgrim_road_length_voxels,
    first_pilgrim_road_point_at_distance, first_pilgrim_route_anchor,
    first_pilgrim_route_anchor_for_feature_cell, generate_edited_surface_tile_mesh,
    generate_edited_water_tile_mesh, generate_far_tile, generate_far_tile_with, mesh_chunk,
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
    let Some((road, _)) = first_pilgrim_road_point_at_distance(road_length * 0.5) else {
        return;
    };
    let road_x = road[0].round() as i32;
    let road_z = road[1].round() as i32;
    let road_y = generator.surface_height(road_x, road_z);
    let road_coord = VoxelCoord::new(road_x, road_y, road_z).chunk();
    criterion.bench_function("generate 32^3 pilgrim-road chunk", |bencher| {
        bencher.iter(|| generator.generate_chunk(road_coord));
    });
}

fn route_surface_lod(criterion: &mut Criterion) {
    let generator = Generator::new(SEED);
    let edits = EditMap::default();
    let road_length = first_pilgrim_road_length_voxels();
    let Some((road, _)) = first_pilgrim_road_point_at_distance(road_length * 0.5) else {
        return;
    };
    let road = [road[0].round() as i32, road[1].round() as i32];
    let mut group = criterion.benchmark_group("pilgrim-road surface LOD");
    for level in [SurfaceLodLevel::Stride2, SurfaceLodLevel::Stride16] {
        let coord = SurfaceTileCoord::containing(level, road[0], road[1]);
        group.bench_function(
            format!("stride-{} tile", level.stride_voxels()),
            |bencher| {
                bencher.iter(|| generate_edited_surface_tile_mesh(generator, &edits, coord));
            },
        );
    }
    group.finish();
}

fn semantic_hero_generation(criterion: &mut Criterion) {
    let generator = Generator::new(SEED);
    let Some(hero) =
        generator.nearest_prominent_skyline_feature(0, 0, SkylineFeatureKind::ElderCanopy, 192)
    else {
        return;
    };
    let hero_chunk = VoxelCoord::new(hero.anchor[0], hero.trunk_top, hero.anchor[2]).chunk();
    criterion.bench_function("generate 32^3 elder-canopy hero chunk", |bencher| {
        bencher.iter(|| generator.generate_chunk(hero_chunk));
    });

    let edits = EditMap::default();
    let mut group = criterion.benchmark_group("semantic-hero surface LOD");
    for level in [SurfaceLodLevel::Stride2, SurfaceLodLevel::Stride16] {
        let coord = SurfaceTileCoord::containing(level, hero.anchor[0], hero.anchor[2]);
        group.bench_function(
            format!("stride-{} elder-canopy tile", level.stride_voxels()),
            |bencher| {
                bencher.iter(|| generate_edited_surface_tile_mesh(generator, &edits, coord));
            },
        );
    }
    group.finish();
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

fn codec(criterion: &mut Criterion) {
    let chunk = Generator::new(SEED).generate_chunk(COORD);
    let encoded = encode_chunk(&chunk);
    let mut group = criterion.benchmark_group("VXCH palette codec");
    group.throughput(criterion::Throughput::Bytes(
        (chunk.voxels().len() * size_of::<u16>()) as u64,
    ));
    group.bench_function("encode", |bencher| {
        bencher.iter(|| encode_chunk(&chunk));
    });
    group.bench_function("decode", |bencher| {
        bencher.iter(|| decode_chunk(&encoded));
    });
    group.finish();
}

fn meshing(criterion: &mut Criterion) {
    let generator = Generator::new(SEED);
    criterion.bench_function("greedy mesh generated chunk", |bencher| {
        bencher.iter_batched(
            || generator.generate_chunk(COORD),
            |chunk| {
                let origin = chunk.coord().world_origin();
                let region = generator.region(origin[0] - 1, origin[2] - 1, 34, 34);
                mesh_chunk(&chunk, |x, y, z| region.sample(x, y, z))
            },
            BatchSize::SmallInput,
        );
    });
}

fn water_meshing(criterion: &mut Criterion) {
    let generator = Generator::new(SEED);
    let coord = OCEAN_VOXEL.chunk();
    criterion.bench_function("greedy mesh generated ocean chunk", |bencher| {
        bencher.iter_batched(
            || generator.generate_chunk(coord),
            |chunk| {
                let origin = chunk.coord().world_origin();
                let region = generator.region(origin[0] - 1, origin[2] - 1, 34, 34);
                mesh_chunk(&chunk, |x, y, z| region.sample(x, y, z))
            },
            BatchSize::SmallInput,
        );
    });
}

fn water_surface_lod(criterion: &mut Criterion) {
    let generator = Generator::new(SEED);
    let edits = EditMap::default();
    let coord =
        SurfaceTileCoord::containing(SurfaceLodLevel::Stride8, OCEAN_VOXEL.x, OCEAN_VOXEL.z);
    criterion.bench_function("generate edit-aware stride-8 water tile", |bencher| {
        bencher.iter(|| generate_edited_water_tile_mesh(generator, &edits, coord));
    });
}

fn far_surface(criterion: &mut Criterion) {
    let generator = Generator::new(SEED);
    criterion.bench_function("generate 25.6m far surface tile", |bencher| {
        bencher.iter(|| generate_far_tile(generator, FarTileCoord::new(2, -3)));
    });
}

fn edited_far_surface(criterion: &mut Criterion) {
    let generator = Generator::new(SEED);
    let coord = FarTileCoord::new(2, -3);
    let mut edits = EditMap::default();
    for index in 0..10_000 {
        edits.insert_override(
            VoxelCoord::new(1_000_000 + index, 80, 1_000_000 - index),
            Material::Stone,
        );
    }
    criterion.bench_function("generate far tile with 10k unrelated edits", |bencher| {
        bencher
            .iter(|| generate_far_tile_with(coord, |x, z| edits.surface_sample(generator, x, z)));
    });
}

criterion_group!(
    world_benches,
    generation,
    route_surface_lod,
    semantic_hero_generation,
    route_queries,
    codec,
    meshing,
    water_meshing,
    water_surface_lod,
    far_surface,
    edited_far_surface
);
criterion_main!(world_benches);
