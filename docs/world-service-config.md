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
schema_version = 17
world_id = "766f7865-6c73-406c-6f63-616c00000001"
world_seed = 1592642302
source = "terrain-diffusion-30m"

[transport]
listen = "127.0.0.1:9777"
allowed_origins = ["http://127.0.0.1:5173", "http://localhost:5173"]
auth_subprotocol_token = "replace-with-a-random-local-token"
max_frame_bytes = 16777216
max_queued_outbound_bytes_per_client = 33554432
outbound_bandwidth_floor_bytes_per_second = 98304
outbound_bandwidth_ceiling_bytes_per_second = 4194304
outbound_bandwidth_burst_bytes = 65536
outbound_queue_delay_target_ms = 25
outbound_feedback_timeout_ms = 3000
max_in_flight_batches = 16
max_connections = 1024
global_queue_capacity = 16384
product_cache_bytes = 268435456
generation_workers = 8
generation_workers_per_client = 2

[presence]
broadcast_interval_ms = 33
max_players = 1024
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
allow_creative_flight = true
interaction_reach_centimetres = 500
interaction_latency_slack_centimetres = 100
interaction_pose_max_age_ms = 1000
max_horizontal_speed_centimetres_per_second = 900
max_vertical_speed_centimetres_per_second = 1200
movement_slack_centimetres = 100
movement_credit_window_ms = 500

[environment]
day_length_seconds = 1200.0
world_day_number_at_unix_epoch = 0
day_fraction_at_unix_epoch = 0.72
days_per_year = 365.2422
moon_sidereal_orbit_days = 27.321661
moon_orbit_phase_at_world_epoch = 0.0
planet_circumference_metres = 40075016.0
axial_tilt_degrees = 23.4393
moon_orbit_inclination_degrees = 5.145
celestial_seed = 1470258925
celestial_revision = 1
weather_cycle_seconds = 900.0
weather_fraction_at_unix_epoch = 0.08
cloud_offset_metres_at_unix_epoch = [0.0, 0.0]
cloud_velocity_metres_per_second = [5.5, 1.6]
cloud_coverage = 0.24
cloud_base_metres = 550.0
cloud_top_metres = 1800.0
weather_seed = 1474984685
weather_revision = 1

[edits]
database = "../tmp/world-state/schema-{edit_schema}/{world_id}-{source_hash}.sqlite3"
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

`outbound_bandwidth_floor_bytes_per_second` is the safe VXWP payload rate shared by a player's world
and presence WebSockets. When the receiver reports end-to-end RTT and the connection has queued
demand, the server may double that rate during initial capacity discovery up to
`outbound_bandwidth_ceiling_bytes_per_second`. After the first congestion signal, recovery uses
smaller 25% probes. It compares the latest RTT with the minimum observed RTT for that session;
growth beyond `outbound_queue_delay_target_ms` halves the rate immediately.
Missing feedback for `outbound_feedback_timeout_ms` restores the floor and clears the learned path
baseline. This delay-based application controller does not replace TCP congestion control: it keeps
our own offered load from maintaining a standing queue above TCP while allowing healthy links to use
more bandwidth.

`outbound_bandwidth_burst_bytes` supplies startup credit. Frames are never split merely to satisfy
the bucket: if one encoded frame is larger than the burst it is sent whole, and its full byte debt
delays later traffic. Control and authoritative acknowledgements preempt other traffic.
Collision/startup data, authoritative world changes, realtime presence, visible terrain, and
prefetch then use weighted, work-conserving service; an idle class gives its capacity to active
classes.

`max_queued_outbound_bytes_per_client` is a separate memory-safety bound on completed world
products. It is not a rate limit.

The absolute world-day anchor is evaluated against Unix time and then transmitted with the server's
monotonic clock, so daemon restarts and multiple clients retain one sky. Year, lunar orbit, local
solar time, and sidereal star rotation all derive from that one clock. Set `day_length_seconds = 0`
to freeze the entire celestial clock for visual testing. The plane maps only its observer frame onto
the configured spherical circumference: `(0, 0)` is the equator, north is `-Z`, east is `+X`, and
continuing through either pole smoothly returns toward the opposite equator. Terrain itself never
wraps. Cloud wind and the continuous weather timeline use the same clock contract. Set
`weather_cycle_seconds = 0` to
freeze `weather_fraction_at_unix_epoch`; otherwise the cycle moves continuously through clear,
cloudy, overcast, rain, storm, and clearing conditions. Coverage is the clear-sky baseline, while
the cloud base/top bound the volumetric layer. The derived weather drives sky color, sunlight,
shadows, fog, cloud density, precipitation, and cold-region snow from one revisioned environment
snapshot without per-frame network messages.

`allow_creative_flight` advertises a world capability and accepts the corresponding player-pose
flag. Flight remains subject to the same horizontal/vertical speed and movement-credit budgets;
disabling it removes the capability and rejects flying poses.

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
the complete local generation pool. `product_cache_bytes` bounds an LRU of validated encoded product
items. Concurrent overlapping batches single-flight each chunk or surface tile through one CPU/Metal
generation, then assemble their requested order into independently request-ID-keyed VXWP responses.
Priority and batch shape affect scheduling, not cache identity.

`[edits].database` is the native authoritative world/player SQLite file. Relative paths resolve from
the service configuration, not the process working directory. The Rust service expands
`{edit_schema}`, `{world_id}`, and `{source_hash}` before opening SQLite. The checked-in path uses all
three, so changing a storage schema or any immutable world/source identity starts fresh local state
and leaves the previous world untouched. This also prevents an old and new daemon from opening the
same filename during hot reload. Paths without tokens are opened exactly as configured and remain
strict: startup rejects another schema or a database bound to a different world/source manifest;
there are no migrations or fallback authorities. Schema 5 owns sparse voxel edits, face-oriented dig
operations, player material inventories, idempotent edit sessions, and authoritative resume poses.
`change_queue_capacity` bounds each interested client's commit queue.

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
a fixed teleport tolerance on every update. New players start with an empty inventory; digging
credits the exact canonical materials removed, and placement debits those earned voxel units.
