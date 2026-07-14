# Native Terrain Diffusion Metal experiment

`world-terrain-diffusion` is an optional macOS-only provider experiment for
[xandergos/terrain-diffusion](https://github.com/xandergos/terrain-diffusion). It ports the
published magnitude-preserving U-Net, portable spatial RNG, DPM++ sampler, base consistency pass,
and decoder consistency pass to Rust. Candle executes learned tensor operations directly through
its Metal backend. There is no Python, PyTorch, CUDA, subprocess inference, HTTP inference, or CPU
inference fallback.

The port retains the upstream MIT notice in `world-terrain-diffusion/UPSTREAM_LICENSE`.

This is native Rust plus Metal, not PyTorch's backend named `mps` and not an MPSGraph binding. The
important runtime property is the same: model tensors and learned operations stay on the Apple GPU.
Model loading fails when Metal is unavailable.

## Reproducibility and model files

The provider pins:

- upstream model repository `xandergos/terrain-diffusion-30m`;
- immutable Hugging Face revision `9ef8030cb805b433b98ec25c5dddefbac07a9e26`;
- SHA-256 for the coarse, base, and decoder safetensors;
- sampler, scheduler, macro-field, coordinate, and voxel-composer versions in
  `WorldSourceIdentity`.

`vp run terrain:fetch` streams the seven pinned files into
`~/Library/Caches/voxels/terrain-diffusion/<revision>`. Each download uses a partial file and atomic
rename; all 1.137 GB of published weights are verified before the path is returned. Inference is
offline after the fetch.

## Running it

```sh
vp run terrain:fetch
vp run terrain:counterproof
vp run terrain:smoke
vp run terrain:base
vp run terrain:detail
```

Set `VOXELS_TERRAIN_SEED` to an unsigned integer to reproduce or compare another generated world;
the seed is part of `WorldSourceIdentity`.

The `metal` Cargo feature is off by default, so the normal host and WASM builds do not compile or
link an ML runtime. Native ML runs use the `worldgen` Cargo profile (`opt-level = 3`, LTO, one codegen
unit) and FP16 model tensors by default. FP32 remains available as a diagnostic precision.

`terrain:counterproof` verifies numerically that every learned stage responds to changed image
conditioning. `terrain:smoke` runs the authentic 20-step coarse path on a 64x64 tile.
`terrain:detail` loads all three models and runs a finite coarse -> two-pass base -> decoder chain,
producing 128x128 elevation at the model's native 30 m spacing. `terrain:base` stops after the first
two stages for independent attention-path validation. The 128-pixel decoder tile matches the
training crop and is the lowest-latency useful detail tile; larger overlap-aware tiles can be added
behind the same API.

On the development M3 Max, an optimized fresh process with warm Metal shader caches measured 482 ms
for coarse diffusion, 211 ms for the two base passes, and 112 ms for decoding. Model loading,
postprocessing, and those learned stages took 1.04 seconds wall-clock in total. Maximum resident set
size was 2.08 GB and peak process footprint was 3.04 GB. The first run after a shader or binary
change can be slower while Metal compiles kernels.

## World-source boundary

`TerrainDiffusionMacroTileSource` implements the portable `world::MacroTerrainSource` trait. It
owns only the generated canonical fields after inference, not Candle or Metal values, which makes
the result suitable for an in-process call today and the planned binary world-service protocol
later. One experimental tile covers 3.84 km square at 30 m per model pixel (300 canonical 10 cm
voxels per pixel); samples beyond it are explicitly invalid rather than silently repeating.

`world-service` loads the versioned [world source configuration](world-service-config.md) and
constructs either this provider or `ProceduralWorldSource` behind that same trait. Provider
selection, model paths, and precision remain server-only; clients consume identical canonical world
products.

The service configuration keeps two coordinate origins explicit. `model_origin` selects the
coordinate-keyed model/noise sample, while `world_origin_voxels` places the resulting finite tile in
canonical voxel X/Z space. Both are bound into `WorldSourceIdentity`, and the latter is published as
the macro coordinate-transform origin, so moving or regenerating a tile cannot reuse an incompatible
cache identity.

This is not yet selected by the browser shell. The shell is WASM and cannot host this native Metal
executor. The remaining production work is the already-planned local world service plus extraction
of the canonical macro-field-to-voxel composer. Those steps turn the finite source into the game's
full `WorldSourceEngine` without moving model code into the browser or changing persisted
procedural-v16 worlds.

## Fidelity and performance notes

- The U-Net appends the learned bias channel, normalizes magnitude-preserving convolution weights
  once at load, uses the published SiLU/residual/concatenation factors, exact pooling/repeat
  resampling, and fused Metal scaled-dot-product attention.
- Coordinate-keyed PCG XSH-RR and Marsaglia polar noise preserves overlap across independently
  requested tiles.
- Coarse generation uses the published five conditioning channels, 20 Karras sigmas, and second
  order midpoint DPM++ updates.
- The base stage uses the exact 58-component conditioning layout and the published `sigma=80` then
  `sigma=0.35` consistency passes. The decoder uses the first four latent bands and retains the
  fifth low-frequency band for signed-square-root elevation reconstruction.
- The current finite experiment uses a uniform +400 m signed-square-root elevation control with an
  elevation conditioning noise ratio of 0.05, while leaving climate controls at their training
  means and published ratio 0.5. The global dataset mean is ocean-heavy, so this makes the finite
  smoke tile exercise land generation without fabricating or offsetting the learned result. It
  selects the highest-mean 4x4 coarse elevation window and then centres its 16x16 latent preview on
  the highest learned low-frequency point. It performs the upstream
  signed-square-root/Laplacian reconstruction in native Rust. Infinite overlap blending and the
  upstream synthetic climate-map generator are intentionally still future fidelity work and are
  part of the source identity when added.
