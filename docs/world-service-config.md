# World service configuration

The world service owns source selection. It loads the versioned TOML file at
`config/world-service.toml`, constructs one `MacroTerrainSource` at startup, and exposes the same
canonical macro-field products regardless of provider. Browser and future network clients do not
read this file and do not branch on the provider.

## Selecting a source

The checked-in default is:

```toml
source = "terrain-diffusion-30m"
```

To use the deterministic authored generator instead, change only that field:

```toml
source = "procedural-v16"
```

Restart the service after changing the file. Selection is intentionally fail-closed: choosing
Terrain Diffusion without its pinned model files, Apple Metal, or the compiled `terrain-metal`
feature is an error. It never silently creates a different procedural world.

The complete schema is:

```toml
schema_version = 11
world_id = "766f7865-6c73-406c-6f63-616c00000001"
world_seed = 1592642302
source = "terrain-diffusion-30m"

[transport]
listen = "127.0.0.1:9777"
allowed_origins = ["http://127.0.0.1:5173", "http://localhost:5173"]
auth_subprotocol_token = "replace-with-a-random-local-token"
max_frame_bytes = 16777216
max_outbound_bytes_per_client = 33554432
max_in_flight_batches = 16
max_connections = 512
global_queue_capacity = 8192
product_cache_bytes = 268435456
generation_workers = 8
generation_workers_per_client = 2

[presence]
broadcast_interval_ms = 33
max_players = 512
max_pose_updates_per_second = 60
spatial_cell_metres = 64
interest_radius_metres = 256
interest_hysteresis_metres = 32
near_radius_metres = 32
mid_radius_metres = 96
near_update_interval_ms = 50
mid_update_interval_ms = 100
far_update_interval_ms = 250
max_records_per_delta = 64
prediction_error_centimetres = 25
look_error_milliradians = 175

[gameplay]
interaction_reach_centimetres = 500
interaction_latency_slack_centimetres = 100
interaction_pose_max_age_ms = 1000
max_horizontal_speed_centimetres_per_second = 900
max_vertical_speed_centimetres_per_second = 1200
movement_slack_centimetres = 100
movement_credit_window_ms = 500

[edits]
database = "../tmp/world-state-v5.sqlite3"
change_queue_capacity = 256

[spawn]
xz_voxels = [0, 0]

[terrain_diffusion]
precision = "float16"
world_origin_voxels = [-76800, -76800]
horizontal_scale = 1
latent_window = [-2, -1]
quality_histogram = [0.0, 0.0, 0.0, 1.0, 1.5]
sea_level_voxels = 52
# model_cache = "/an/optional/cache/root"
```

`float16` is the high-performance default; `float32` is available for diagnostics. If
`model_cache` is omitted on macOS, the service loads the immutable pinned revision from
`~/Library/Caches/voxels/terrain-diffusion/<revision>`. A relative path is resolved relative to the
configuration file, not the process working directory. The seed, precision, horizontal scale,
latent window, and quality histogram participate in the source identity used by caches and future
protocol negotiation.

`world_origin_voxels` is the canonical voxel X/Z coordinate of the finite generated tile's minimum
corner. `horizontal_scale = 1` maps each native 30 m model pixel across 30 m of world space. This
preserves physical slopes because canonical voxels have a fixed 10 cm size; Minecraft's recommended
scale changes both axes through the size represented by one cubic block.
`[-76800, -76800]` therefore centers the 15.36 km tile on the default `[0, 0]` spawn and leaves room
for the exact one-voxel meshing halo around spawn. The server validates the actual spawn chunk at startup.
`latent_window` is the Terrain Diffusion latent-window row/column used to key spatial sampling and
noise. Each step advances 32 latent pixels, or 7.68 km for the 30 m checkpoint.
The checked-in `[-2, -1]` window is a fixed, reproducible showcase start for the checked-in seed: it
puts the spawn on land amid the rugged fjord and plateau structure learned by the model. Runtime
generation never searches for or substitutes a more dramatic window.
`quality_histogram` is the five-bin learned terrain-quality conditioning vector. The checked-in
`[0, 0, 0, 1, 1.5]` preset is the upstream showcase setting and favors the two highest-rated bins;
all zeros selects the unsteered checkpoint default.
Model sampling is deliberately separate from world placement: moving an unchanged tile in the game
world is not the same operation as generating a different model-space tile. Both origins participate
in the source identity, and `world_origin_voxels` is also declared in the macro coordinate transform.
Changing either value therefore creates a distinct cache/protocol identity instead of aliasing
existing world products.

Both coordinate values must contain exactly two signed 32-bit integers. Unknown keys, malformed values,
and unsupported `schema_version` values are rejected so configuration mistakes cannot silently alter
a world. Model repository/revision, verified configuration and weight hashes, tensor shapes,
normalization statistics, and sampler/scheduler semantics remain pinned provider invariants rather
than deployment settings.

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
the complete local generation pool. `product_cache_bytes` bounds an LRU of compressed immutable
batch responses. Concurrent identical batches single-flight through one CPU/Metal generation and
then receive independently request-ID-keyed copies of that response.

`[edits].database` is the native authoritative world/player SQLite file. Relative paths resolve from
the service configuration, not the process working directory. Startup initializes only schema 5 and
rejects another schema or a database bound to a different world/source manifest; there are no
migrations or fallback authorities. Schema 5 owns sparse voxel edits, face-oriented dig operations,
player material inventories, idempotent edit sessions, and authoritative resume poses. The v5
filename leaves older local worlds untouched. `change_queue_capacity` bounds each interested
client's commit queue.

The presence section controls the independent low-latency delta stream. `spatial_cell_metres`,
`interest_radius_metres`, and `interest_hysteresis_metres` define receiver-specific interest without
splitting the world: every player at the same location remains in the same authoritative simulation.
The near/mid/far radii and intervals reduce freshness with distance. Prediction and look-error
thresholds can promote a correction before its interval, while age always accumulates so a dense
crowd cannot starve. `max_records_per_delta` is the hard per-receiver dense-region budget shared by
enters and pose updates. `max_players` and `max_pose_updates_per_second` bound state and inbound work.

The gameplay section is server-only authority. Interaction reach is 5 m with a hard 1 m allowance
for ordinary ordering skew between the dedicated presence and world sockets; edits are rejected when
the pose is stale. Horizontal and vertical token buckets replenish at their configured speeds and
retain only bounded slack/window credit, so packet delay is tolerated without letting a client reuse
a fixed teleport tolerance on every update. New players receive the configured count for every
non-Air material; digging and placement then credit and debit exact canonical-voxel units.
