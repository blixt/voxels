#![allow(
    clippy::expect_used,
    reason = "invalid deterministic benchmark fixtures must fail instead of silently measuring a no-op"
)]

use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use std::hint::black_box;
use voxels_runtime::{
    CompletionStatus, FrameBudget, MAX_SECONDARY_INTEREST_CHUNKS, StreamConfig, StreamScheduler,
};
use voxels_world::ChunkCoord;

const CONFIG: StreamConfig = StreamConfig {
    load_radius_chunks: 5,
    vertical_radius_chunks: 1,
    retention_margin_chunks: 1,
    max_tracked_chunks: 320,
    max_secondary_interest_chunks: MAX_SECONDARY_INTEREST_CHUNKS,
};
const BUDGET: FrameBudget = FrameBudget {
    generation: 2,
    meshing: 1,
    upload: 3,
};
const FOCUS: ChunkCoord = ChunkCoord::new(0, 1, 0);

fn frame_admission(criterion: &mut Criterion) {
    criterion.bench_function("populate 243-chunk cold interest set", |bencher| {
        bencher.iter_batched(
            || StreamScheduler::new(CONFIG).expect("benchmark stream config must remain valid"),
            |mut scheduler| {
                scheduler.update_focus(FOCUS);
                assert_eq!(scheduler.diagnostics().tracked, 243);
                black_box(scheduler);
            },
            BatchSize::SmallInput,
        );
    });

    criterion.bench_function("admit one bounded streaming frame", |bencher| {
        bencher.iter_batched(
            || {
                let mut scheduler = StreamScheduler::new(CONFIG)
                    .expect("benchmark stream config must remain valid");
                scheduler.update_focus(FOCUS);
                let generation = scheduler.schedule_frame(FrameBudget {
                    generation: 5,
                    ..FrameBudget::default()
                });
                assert_eq!(generation.generation.len(), 5);
                for ticket in generation.generation {
                    assert_eq!(scheduler.complete(ticket), CompletionStatus::Accepted);
                }
                let meshing = scheduler.schedule_frame(FrameBudget {
                    meshing: 3,
                    ..FrameBudget::default()
                });
                assert_eq!(meshing.meshing.len(), 3);
                for ticket in meshing.meshing {
                    assert_eq!(scheduler.complete(ticket), CompletionStatus::Accepted);
                }
                scheduler
            },
            |mut scheduler| {
                let work = scheduler.schedule_frame(BUDGET);
                assert_eq!(
                    [work.generation.len(), work.meshing.len(), work.upload.len()],
                    [2, 1, 3]
                );
                black_box(work);
            },
            BatchSize::SmallInput,
        );
    });
}

criterion_group!(streaming_benches, frame_admission);
criterion_main!(streaming_benches);
