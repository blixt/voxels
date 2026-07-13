use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use std::hint::black_box;
use voxels_runtime::{FrameBudget, StreamConfig, StreamScheduler};
use voxels_world::ChunkCoord;

const CONFIG: StreamConfig = StreamConfig {
    load_radius_chunks: 5,
    vertical_radius_chunks: 1,
    retention_margin_chunks: 1,
    max_tracked_chunks: 320,
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
            || StreamScheduler::new(CONFIG),
            |scheduler| {
                let Ok(mut scheduler) = scheduler else {
                    return;
                };
                scheduler.update_focus(FOCUS);
            },
            BatchSize::SmallInput,
        );
    });

    criterion.bench_function("admit one bounded streaming frame", |bencher| {
        bencher.iter_batched(
            || {
                let Ok(mut scheduler) = StreamScheduler::new(CONFIG) else {
                    return None;
                };
                scheduler.update_focus(FOCUS);
                let generation = scheduler.schedule_frame(FrameBudget {
                    generation: 5,
                    ..FrameBudget::default()
                });
                assert_eq!(generation.generation.len(), 5);
                for ticket in generation.generation {
                    let _ = scheduler.complete(ticket);
                }
                let meshing = scheduler.schedule_frame(FrameBudget {
                    meshing: 3,
                    ..FrameBudget::default()
                });
                assert_eq!(meshing.meshing.len(), 3);
                for ticket in meshing.meshing {
                    let _ = scheduler.complete(ticket);
                }
                Some(scheduler)
            },
            |scheduler| {
                if let Some(mut scheduler) = scheduler {
                    let work = scheduler.schedule_frame(BUDGET);
                    assert_eq!(
                        [work.generation.len(), work.meshing.len(), work.upload.len()],
                        [2, 1, 3]
                    );
                    black_box(work);
                }
            },
            BatchSize::SmallInput,
        );
    });
}

criterion_group!(streaming_benches, frame_admission);
criterion_main!(streaming_benches);
