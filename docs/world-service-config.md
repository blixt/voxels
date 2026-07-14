# World service configuration

The world service owns source selection. It loads the versioned TOML file at
`config/world-service.toml`, constructs one `MacroTerrainSource` at startup, and exposes the same
canonical macro-field products regardless of provider. Browser and future network clients do not
read this file and do not branch on the provider.

## Selecting a source

Change this one line in `config/world-service.toml`:

```toml
source = "procedural-v16"
```

or:

```toml
source = "terrain-diffusion-30m"
```

Restart the service after changing the file. Selection is intentionally fail-closed: choosing
Terrain Diffusion without its pinned model files, Apple Metal, or the compiled `terrain-metal`
feature is an error. It never silently creates a different procedural world.

The complete schema is:

```toml
schema_version = 5
world_id = "766f7865-6c73-406c-6f63-616c00000001"
world_seed = 1592642302
source = "procedural-v16"

[transport]
listen = "127.0.0.1:9777"
allowed_origins = ["http://127.0.0.1:5173", "http://localhost:5173"]
auth_subprotocol_token = "replace-with-a-random-local-token"
max_frame_bytes = 16777216
max_outbound_bytes_per_client = 33554432
max_in_flight_batches = 16
max_connections = 32
global_queue_capacity = 128
generation_workers = 8
generation_workers_per_client = 2

[presence]
broadcast_interval_ms = 33
max_players = 32
max_pose_updates_per_second = 60
teleport_distance_metres = 4

[spawn]
xz_voxels = [0, 0]

[terrain_diffusion]
precision = "float16"
world_origin_voxels = [-19200, -19200]
model_origin = [0, 0]
sea_level_voxels = 52
# model_cache = "/an/optional/cache/root"
```

`float16` is the high-performance default; `float32` is available for diagnostics. If
`model_cache` is omitted on macOS, the service loads the immutable pinned revision from
`~/Library/Caches/voxels/terrain-diffusion/<revision>`. A relative path is resolved relative to the
configuration file, not the process working directory. The seed and precision participate in the
source identity used by caches and future protocol negotiation.

`world_origin_voxels` is the canonical voxel X/Z coordinate of the finite generated tile's minimum
corner. `[-19200, -19200]` centers the 3.84 km tile on the default `[0, 0]` spawn and leaves room for
the exact one-voxel meshing halo around spawn. The server validates the actual spawn chunk at startup.
`model_origin` is the Terrain Diffusion model-grid row/column used to key spatial sampling and noise.
They are deliberately separate: moving an unchanged tile in the game world is not the same operation
as generating a different model-space tile. Both origins participate in the source identity, and
`world_origin_voxels` is also declared in the macro coordinate transform. Changing either value
therefore creates a distinct cache/protocol identity instead of aliasing existing world products.

Both origin values must contain exactly two signed 32-bit integers. Unknown keys, malformed values,
and unsupported `schema_version` values are rejected so configuration mistakes cannot silently alter
a world. Model repository/revision, verified weight hashes, tensor shapes, normalization statistics,
and sampler/scheduler semantics remain pinned provider invariants rather than deployment settings.

## Checking the configured provider

Fetch and verify the model once if Terrain Diffusion will be selected:

```sh
vp run terrain:fetch
```

Then load the configuration, construct the selected provider, and request one canonical macro
sample:

```sh
vp run world:source-smoke
```

The command reports the selected source kind and stable source-identity hash. In Terrain Diffusion
mode it runs the full coarse, base, and decoder chain natively in Rust on Metal.

The browser always consumes the daemon's canonical chunk and progressive surface-LOD products. The
client never branches on provider selection; changing the daemon source and restarting it is
sufficient to switch between procedural and learned terrain. The transport bounds total accepted
connections and queued work, and the per-client worker cap prevents one connection from occupying
the complete local generation pool.

The presence section controls the independent low-latency roster stream. `broadcast_interval_ms`
coalesces all accepted latest poses into complete snapshots; `max_players` and
`max_pose_updates_per_second` bound memory and client work; `teleport_distance_metres` requires large
motion to carry an explicit discontinuity instead of being interpolated through the terrain.
