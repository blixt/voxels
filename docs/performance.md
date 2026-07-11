# Performance baselines

Performance changes must keep a reproducible before/after number beside the architectural reason for
the change. Native Criterion results are stable microbenchmarks for portable Rust algorithms; browser
Mission Control and `window.__VOXELS__.snapshot()` cover integrated frame pacing and residency.

## 2026-07-11: streamed chunk preparation

Measured on an Apple M3 Max with Rust 1.97.0 using the release-profile command below. Criterion used
100 samples after its standard warm-up.

```sh
vp run bench:world
```

| Operation                        |   Before |    After | Change |
| -------------------------------- | -------: | -------: | -----: |
| Generate one canonical 32³ chunk | 2.795 ms | 0.600 ms | -78.5% |
| Greedy mesh one generated chunk  | 1.517 ms | 1.470 ms |  -3.1% |

Generation previously recalculated height and biome noise for every Y voxel. The optimized path
computes an immutable profile once for each of the chunk's 1,024 X/Z columns, then uses it while
filling all 32 layers. A host test compares stratified generated voxels with the authoritative
random-access sampler, so the optimization does not change generator identity or saved edits.

Meshing now stores its short-lived 34³ solid halo as bytes instead of `Vec<bool>` proxy bits. This is
a small speed win and keeps each hot solidity read branch-free.

The worker admits two generation tickets per display frame after this change. Their measured native
CPU budget is about 1.20 ms combined, less than half the former one-ticket cost of 2.80 ms, while
doubling initial residency throughput. Meshing and GPU upload remain independently bounded, so a
large focus jump cannot enqueue unbounded same-frame work.

Integrated browser validation with three 1024² sun-shadow cascades held the 120 Hz display cap in the
tested spawn view. Mission Control reported 51 main draws plus 299 light-volume-culled shadow draws;
turning shadows off reduced the latter to zero without changing renderer layouts.

These numbers are a decision record, not universal hardware targets. Criterion's stored baselines in
`target/criterion/` remain ignored and local; repeat the command on target hardware before changing
per-frame admission budgets.
