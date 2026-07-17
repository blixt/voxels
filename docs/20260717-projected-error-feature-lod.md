# Projected-error feature LOD

## Decision

Keep one CPU-authored ownership system, but stop making standalone world features inherit the
terrain patch's LOD. Terrain, water, and features are different render products with different
error models. The server should stream each product progressively; the client should select one
resident representation using measured screen-space error.

Do not add another shader-side radial owner. A short stochastic transition may conceal the final
sub-pixel residual, but it must not decide ownership or cover missing geometry.

## Why the current handoff is visible

The current hierarchy has valuable correctness properties: one selected terrain owner, complete
sibling replacement, exact height-matched connectors, and resident-parent fallback. Its remaining
fidelity problem is granularity:

- `SurfacePatch.quad_range` contains both terrain and skyline proxies, so a tree changes whenever
  its 8-by-8-cell terrain patch changes.
- Terrain uses fixed world-space rings. At a 68 degree vertical field of view and 720p, their
  nominal geometric steps project to roughly 5.6 pixels at the canonical/stride-2 boundary and
  4.2 pixels at later boundaries. A tall asymmetric tree can move farther than one terrain step.
- Proxies used to be independently authored boxes. They are now derived from the canonical tree
  and preserve aggregate area, but a fixed ring still cannot account for feature size, silhouette
  complexity, or the camera's projection.

Nanite's important lesson is not a longer cross-fade. It is fine-grained, same-source
simplification with an object-space error bound, followed by projected-error selection. Foliage
also needs aggregate area preservation so simplification does not make crowns thin out.

## Target wire product

Introduce one versioned `FeatureLodProduct` and cut the protocol directly to it; this project does
not need a compatibility decoder or migration.

```text
FeatureLodProduct
  world/source identity
  feature id + feature revision
  conservative world bounds
  material-area summary
  levels[] (coarse first)
    packed quads or meshlets
    object-space error in metres
    conservative bounds
    content fingerprint
```

Terrain patches must contain terrain only. A feature ID is owned exactly once, independently of
which terrain tile transported its descriptor. Editing any canonical voxel inside a feature's
analytic bounds increments that feature revision and suppresses or rebuilds all of its aggregate
levels atomically.

The server sends the coarsest useful feature level with the coarse terrain response, then streams
refinements in descending projected-error benefit per byte. Products are immutable and cacheable
by source identity, feature ID, revision, and level. This is compatible with local IPC and the
future network protocol without moving camera-specific selection onto the server.

## One client selector

For every resident feature, compute:

```text
projected_error_pixels = object_error_metres * focal_length_pixels / view_distance_metres
```

Choose the coarsest resident level below the quality target. Start with a 0.7 pixel target, refine
above 0.9 pixels, and coarsen below 0.55 pixels. The unequal thresholds provide temporal
hysteresis without changing the spatial answer. Very small features may use an area-weighted
coverage threshold before disappearing.

When the desired refinement is absent, retain the current coarser level and raise its stream
priority. Never hide a resident level while waiting for another. When both levels are resident,
draw a bounded world-stable stochastic transition for a few frames only if their measured residual
is still visible. Both levels use the same filtered material and macro-normal inputs during that
transition.

Terrain continues to use the existing hierarchy and exact connectors until it also gains measured
cluster errors. There is still only one source of ownership truth: the CPU draw plan selects
terrain patches, feature levels, water, and connectors before encoding the frame.

## Performance shape

- Generate and cache all feature levels with the world product, not per client.
- Cull one conservative feature bound before testing levels.
- Pack coarse levels contiguously and coalesce adjacent selected ranges into the existing draw
  spans. Move to GPU indirect/meshlet culling only after CPU selection is measured as a bottleneck.
- Preserve trunks and major branches at every tree level. Aggregate foliage into canonical-derived
  volumes with material-area conservation; never replace it with an unrelated billboard.
- Use projected benefit per compressed byte for stream priority. A distant coarse tree costs less
  than a fine terrain patch but remains identifiable much farther away.

## Cutover order

1. Add feature-only ranges/products and remove features from terrain patch ranges in the same
   protocol version bump.
2. Build canonical-derived feature levels with explicit object-space silhouette and area errors.
3. Replace fixed-ring feature ownership with the single hysteretic projected-error selector.
4. Add coarse-first feature streaming and retain-current-level fallback.
5. Add the short world-stable stochastic transition only for residual error that survives the
   structural cutover.
6. If draw submission becomes material, group feature levels into bounded meshlets and use GPU
   indirect compaction without changing the ownership contract.

## Non-negotiable gates

- Exactly one terrain owner and one feature owner at every sampled world point; zero uncovered
  frames during refinement, eviction, reconnect, or HMR.
- Per species and variation: adjacent silhouette IoU, projected contour displacement, area ratio,
  centroid shift, and bounded quads/bytes.
- Same-session browser crossings: registered image SSIM and catastrophic-pixel fraction, including
  a tree-filled boundary and a missing-refinement fallback.
- Stream simulation: coarse-visible time, settled-viewport time, and bytes by product/priority.
- Chromium GPU timestamps: world median/p95, draw spans, selected slices, and 120 Hz missed frames.

The current canonical-derived tree proxy work is the first prerequisite: it removes the unrelated
silhouette and raises worst canonical-to-near-proxy IoU. It is not a substitute for this cutover.

## References

- [Nanite virtualized geometry overview](https://dev.epicgames.com/documentation/en-us/unreal-engine/nanite-virtualized-geometry-in-unreal-engine)
- [Nanite technical details and Preserve Area](https://dev.epicgames.com/documentation/unreal-engine/nanite-technical-details?lang=en-US)
- [Nanite foliage guidance](https://dev.epicgames.com/documentation/en-us/unreal-engine/nanite-foliage)
- [A Deep Dive into Nanite Virtualized Geometry](https://advances.realtimerendering.com/s2021/Karis_Nanite_SIGGRAPH_Advances_2021_final.pdf)
