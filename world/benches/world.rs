use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use std::collections::BTreeMap;
use voxels_world::codec::{decode_chunk, encode_chunk};
use voxels_world::{
    ChunkCoord, EditMap, FarTileCoord, Generator, Material, VoxelCoord, generate_far_tile,
    generate_far_tile_with, mesh_chunk,
};

const SEED: u64 = 0x5eed_cafe;
const COORD: ChunkCoord = ChunkCoord::new(2, 0, -3);

fn generation(criterion: &mut Criterion) {
    let generator = Generator::new(SEED);
    criterion.bench_function("generate 32^3 chunk", |bencher| {
        bencher.iter(|| generator.generate_chunk(COORD));
    });
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
                let mut columns = BTreeMap::new();
                mesh_chunk(&chunk, |x, y, z| {
                    columns
                        .entry((x, z))
                        .or_insert_with(|| generator.column(x, z))
                        .sample(y)
                })
            },
            BatchSize::SmallInput,
        );
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
    codec,
    meshing,
    far_surface,
    edited_far_surface
);
criterion_main!(world_benches);
