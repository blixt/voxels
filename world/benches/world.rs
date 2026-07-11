use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use voxels_world::codec::{decode_chunk, encode_chunk};
use voxels_world::{ChunkCoord, FarTileCoord, Generator, generate_far_tile, mesh_chunk};

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
            |chunk| mesh_chunk(&chunk, |x, y, z| generator.sample(x, y, z)),
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

criterion_group!(world_benches, generation, codec, meshing, far_surface);
criterion_main!(world_benches);
