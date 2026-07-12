use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use glam::Vec3;
use std::hint::black_box;
use voxels_core::{CameraState, InputState, PLAYER_EYE_HEIGHT_METRES, VoxelPhysics};

const STEP: f32 = 1.0 / 120.0;

fn forward_input() -> InputState {
    let mut input = InputState::default();
    input.set_key(1, true);
    input
}

fn bench_camera_update(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("camera_fixed_step_120_ticks");
    group.bench_function("dry_ground", |bencher| {
        bencher.iter_batched(
            || {
                let mut camera = CameraState::spawn(Vec3::new(0.0, PLAYER_EYE_HEIGHT_METRES, 0.0));
                camera.grounded = true;
                (camera, forward_input())
            },
            |(mut camera, input)| {
                for _ in 0..120 {
                    camera.update(&input, STEP, 0.1, |_, y, _| {
                        if y < 0 {
                            VoxelPhysics::SOLID
                        } else {
                            VoxelPhysics::EMPTY
                        }
                    });
                }
                black_box(camera.position)
            },
            BatchSize::SmallInput,
        );
    });
    group.bench_function("submerged", |bencher| {
        bencher.iter_batched(
            || {
                let mut camera = CameraState::spawn(Vec3::new(0.0, 0.56, 0.0));
                camera.refresh_fluid_state(0.1, |_, y, _| {
                    if y < -11 {
                        VoxelPhysics::SOLID
                    } else if y <= 10 {
                        VoxelPhysics::FLUID
                    } else {
                        VoxelPhysics::EMPTY
                    }
                });
                (camera, forward_input())
            },
            |(mut camera, input)| {
                for _ in 0..120 {
                    camera.update(&input, STEP, 0.1, |_, y, _| {
                        if y < -11 {
                            VoxelPhysics::SOLID
                        } else if y <= 10 {
                            VoxelPhysics::FLUID
                        } else {
                            VoxelPhysics::EMPTY
                        }
                    });
                }
                black_box(camera.position)
            },
            BatchSize::SmallInput,
        );
    });
    group.finish();
}

criterion_group!(benches, bench_camera_update);
criterion_main!(benches);
