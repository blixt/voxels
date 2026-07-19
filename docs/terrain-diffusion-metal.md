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
- SHA-256 for the root pipeline configuration and every stage configuration;
- SHA-256 for the coarse, base, and decoder safetensors;
- SHA-256 for the bundled 64-knot ETOPO/WorldClim quantile tables;
- sampler, scheduler, macro-field, coordinate, and voxel-composer versions in
  `WorldSourceIdentity`.

`vp run automation -- run terrain-fetch` streams the seven pinned files into
`~/Library/Caches/voxels/terrain-diffusion/<revision>`. Each download uses a partial file and atomic
rename; all 1.137 GB of published weights are verified before the path is returned. Inference is
offline after the fetch.

## Running it

```sh
vp run automation -- run terrain-fetch
vp run automation -- run terrain-diffusion --mode=counterproof
vp run automation -- run terrain-diffusion
vp run automation -- run terrain-diffusion --mode=base
vp run automation -- run terrain-diffusion --mode=detail
vp run automation -- run terrain-diffusion --mode=survey
```

Set `VOXELS_TERRAIN_SEED` to an unsigned integer to reproduce or compare another generated world;
the seed is part of `WorldSourceIdentity`.

The `metal` Cargo feature is off by default, so the normal host and WASM builds do not compile or
link an ML runtime. Model fetching is a separate `download` feature used only by the diagnostic CLI;
the world service only loads the configured pinned cache and therefore does not compile HTTP/TLS.
Native diagnostic runs use the `worldgen` Cargo profile (`opt-level = 3`, LTO, one codegen unit) and
FP16 model tensors by default. `vp dev` uses the incremental `worldgen-dev` profile while retaining
optimized third-party numerical kernels. FP32 remains available as a diagnostic precision.

`terrain:counterproof` verifies numerically that every learned stage responds to changed image
conditioning. `terrain:smoke` runs the authentic 20-step coarse path on a 64x64 tile.
`terrain:detail` loads all three models and runs a finite coarse -> two-pass base -> decoder chain,
producing 512x512 elevation at the model's native 30 m spacing. `terrain:base` stops after the first
two stages for independent attention-path validation. `terrain:survey` compares coordinate-stable
latent windows without changing the runtime selection. The 512-pixel decoder window matches the
paper and upstream streaming runtime and consumes the complete 64x64 latent result instead of a
hand-selected 16x16 crop. Overlap-aware neighboring windows can be added behind the same API.

On the development M3 Max, an optimized fresh process with warm Metal shader caches measured 409 ms
for coarse diffusion, 183 ms for the two base passes, and 1.55 seconds for the full 512px decoder.
Including model loading and process startup took 4.85 seconds wall-clock. Maximum resident set size
was 2.08 GB and peak process footprint was 6.11 GB. The first run after a shader or binary change can
be slower while Metal compiles kernels.

## World-source boundary

`TerrainDiffusionMacroTileSource` implements the portable `world::MacroTerrainSource` trait. It
owns only the generated canonical fields after inference, not Candle or Metal values, which makes
the result suitable for an in-process call today and the planned binary world-service protocol
later. One experimental tile contains 15.36 km of native model coverage. The checked-in
`horizontal_scale = 1` preserves the model's 30 m physical spacing (300 canonical 10 cm voxels per
pixel); samples beyond it are explicitly invalid rather than silently repeating. Minecraft's
recommended scale changes both horizontal and vertical metres per cubic block. Stretching only the
horizontal axes in our fixed-10 cm world instead flattened the learned slopes.

`world-service` loads the versioned [world source configuration](world-service-config.md) and
constructs either this provider or `ProceduralWorldSource` behind that same trait. Provider
selection, model paths, and precision remain server-only; clients consume identical canonical world
products.

The service configuration keeps model sampling and world placement explicit. `latent_window`
selects a coordinate-keyed 15.36 km latent window on the paper's 7.68 km stride, while
`world_origin_voxels` places the resulting finite tile in canonical voxel X/Z space. Both are bound
into `WorldSourceIdentity`, and the latter is published as the macro coordinate-transform origin, so
moving or regenerating a tile cannot reuse an incompatible cache identity.

This is the checked-in default source for the native world service. The WASM shell never hosts the
Metal executor or branches on provider choice: the service composes learned macro fields into the
same canonical chunks and progressive surface products consumed for procedural worlds. Set
`source = "procedural-v16"` to compare the deterministic authored generator without changing the
client.

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
- The finite experiment ports upstream's FastNoiseLite Perlin FBm and its quantile-matched
  ETOPO/WorldClim prior for all five published conditioning channels. It retains upstream's lapse
  rate, temperature-seasonality, precipitation-variability, and signed-square-root corrections,
  then standardizes output channels `[0, 2, 3, 4, 5]`. The learned model still owns the 30 m terrain.
  The provider directly maps `latent_window` to its centered 4x4 coarse context, uses the
  checkpoint's `0.5` coarse conditioning SNR, and preserves the complete 64x64 learned latent window
  before performing signed-square-root/Laplacian reconstruction in native Rust. Learned coarse
  temperature and precipitation remain spatial fields; the adapter applies elevation lapse rate and
  an aridity estimate before publishing normalized climate. Infinite overlap blending remains future
  work and is source-identity versioned when added.
- The canonical composer adds a deterministic, source-identity-bound subgrid relief signal after
  model inference. Its amplitude follows physical presented slope and stays below 6 m even on steep
  terrain, so it breaks up 30 m bilinear planes without replacing the learned macro shape. Climate,
  altitude, slope, and coherent geology choose biome surfaces and shallow/deep strata. Chunks,
  collision blocks, edited surfaces, and far LODs all use the same composition function.

`terrain:survey` compares coordinate-stable latent windows without changing runtime selection. The
checked-in `[-2, -1]` window for seed `1592642302` is 88% land after decoding, places the centered
spawn at roughly 132 m, and contains rugged fjord and plateau structure with about 115 m median
relief per 960 m diagnostic block. `world:source-smoke` samples the configured tile and reports its
height, climate, ridge, region, and material ranges. These are regression diagnostics rather than
promises that every seed has the same histogram.
