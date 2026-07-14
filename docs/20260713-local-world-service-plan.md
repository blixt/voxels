# Local world-service split plan

Status: implementation handoff  
Scope: one development machine and one local world authority  
Future constraint: the same semantic protocol must be usable over a network for multiplayer  
Primary target: Rust throughout, with an optional native Terrain Diffusion provider accelerated on
Apple silicon

## Outcome

Move canonical world generation and world edits out of the game worker into a native Rust sidecar,
`voxels-worldd`. The game becomes a client of a revisioned world-data service. The first deployment is
strictly local; it is not a cloud or multiplayer implementation. The service boundary must nevertheless
avoid assumptions that prevent a later authoritative multiplayer server.

The service must be able to select either of these sources through world metadata, without changing the
client protocol:

1. `procedural-v16`: the existing deterministic Rust generator.
2. `terrain-diffusion-30m`: a Rust implementation of Terrain Diffusion whose learned stages run on the
   Apple GPU, followed by the same voxel composition, authored routes, caves, materials, and landmarks.

The client must never know which source produced a chunk. It receives authoritative chunk snapshots,
surface-LOD products, world identity, and ordered edit notifications.

## Non-goals

- Do not design cloud deployment, accounts, matchmaking, or public-server security yet.
- Do not move player simulation, rendering, input, or camera persistence into the world service.
- Do not expose per-voxel generation as a network RPC.
- Do not make renderer types part of the protocol.
- Do not require Terrain Diffusion to be complete before the process split ships.
- Do not silently fall back from Metal to CPU when a Terrain Diffusion world requires acceleration.

## Current coupling that must be removed

The shell currently calls the concrete `Generator` in several latency-sensitive paths:

- chunk generation, followed by local `EditMap` application;
- random-access halo sampling during chunk meshing;
- surface and water LOD generation;
- collision, fluid, spawn, and visibility probes;
- deciding whether an edit differs from the generated material;
- persistence identity through the compile-time generator version.

Simply replacing `Generator::sample` with an IPC call would be catastrophically chatty. The split must
move the authority at a data-product boundary: chunks, bounded sample blocks, surface tiles, edits, and
revisions.

## Target process layout

```text
browser game worker
  simulation + interest scheduling + chunk cache + meshing + rendering
                         |
                         | binary world protocol
                         | loopback WebSocket initially
                         v
native voxels-worldd process
  session authority + revisions + edits + persistence + caches + work scheduling
                         |
              +----------+-----------+
              |                      |
       ProceduralSource       TerrainDiffusionSource
       current Rust rules     Rust model + Metal GPU
              |                      |
              +----------+-----------+
                         |
        shared voxel composer and authored overlays
```

For a future native game shell, add a Unix-domain-socket transport and an optional shared-memory bulk
path. The semantic messages and payload codecs must be identical across WebSocket, Unix socket, and a
future network transport.

The browser cannot access Unix sockets or process-shared memory. Therefore the current WASM shell must
use a binary loopback WebSocket served directly by `voxels-worldd`. Do not introduce a JavaScript
world-data layer; Rust in the WASM worker should encode and decode the protocol.

## Ownership boundary

### The world service owns

- immutable world manifest and source identity;
- base-world generation;
- authoritative edit journal and monotonically increasing world revision;
- application of edits to returned chunk and surface products;
- macro-field, generated-chunk, and derived-surface caches;
- terrain-source scheduling, batching, cancellation, and backpressure;
- source-specific model weights and GPU state;
- source-aware validation of persisted data;
- ordered change notifications to connected clients.

### The game client owns

- camera and local preferences;
- player simulation for the current single-player milestone;
- chunk interest and render priority;
- resident authoritative chunk snapshots;
- near-field meshing and GPU uploads;
- renderer resources and disposable render caches;
- reconnect logic and stale-response rejection.

For multiplayer, player simulation may later move server-side, but this plan must not do that work.

## Crate and package layout

Add these workspace members incrementally:

```text
world/                       canonical types, codecs, composition, source traits
world-protocol/              transport-neutral messages and framing
world-client/                request tracking, cache/revision logic, transport trait
world-service/               authority, scheduler, edits, persistence, provider orchestration
world-terrain-diffusion/     optional Candle/Metal model implementation
worldd/                      native executable and local transports
```

Keep the current procedural implementation in `world` initially. It already satisfies the portable,
host-testable constraint. `world-service` wraps it as a provider. The Terrain Diffusion crate depends on
`world`, never the reverse, so normal host tests and builds do not acquire an ML runtime.

`shell` depends on `world-client` and protocol DTOs, but must eventually stop constructing `Generator`
or owning the edit journal.

Use a server-specific release profile with `opt-level = 3`, LTO, and one codegen unit. Do not inherit the
workspace's size-oriented `opt-level = "s"` for the native generation daemon without benchmarking it.

## Swappable source interfaces

Use two boundaries rather than one enormous generator trait.

### 1. Macro terrain

```rust
pub trait MacroTerrainSource: Send + Sync {
    fn identity(&self) -> &MacroSourceIdentity;
    fn request_blocks(&self, request: MacroBlockBatch) -> MacroBlockFuture;
}
```

A macro block contains continuous, versioned fields over an explicit X/Z extent and stride:

- elevation;
- temperature;
- moisture or precipitation;
- optional seasonality and other source channels;
- validity mask and coordinate transform.

The existing provider computes equivalent fields from deterministic noise. Terrain Diffusion produces
them from its hierarchical diffusion pipeline. Field units and interpolation rules belong to the schema,
not to either provider.

### 2. Canonical world products

```rust
pub trait WorldSourceEngine: Send + Sync {
    fn identity(&self) -> &WorldSourceIdentity;
    fn generate_batch(&self, request: WorldProductBatch) -> WorldProductFuture;
}
```

`WorldProductBatch` can request chunks, chunk halos, or surface tiles. The engine obtains macro blocks,
then applies the shared voxel composer, material rules, water, roads, caves, and analytic landmarks.

The exact Rust signatures may use associated futures or provider actors instead of boxed futures. Preserve
batching and cancellation; do not optimize for a pretty one-coordinate-at-a-time trait.

### Source identities

Replace the assumption that `GENERATOR_VERSION` alone identifies every possible world. A world manifest
must contain a stable identity similar to:

```text
world UUID
world seed
world schema version
material schema version
source kind
source implementation version
source configuration hash
model repository and immutable revision, if any
weight hashes, if any
sampler and scheduler version
macro-field schema and coordinate transform
voxel-composer version
authored-content/atlas version
```

Hash the canonical encoding of this identity. Include that hash in cached products and validate it on
load. Never identify a model by a mutable Hugging Face branch name.

## World protocol

Define the schema in `world-protocol`; generate Rust types with `prost`. Use Protocol Buffers for evolving
metadata and opaque, versioned binary payloads for bulk data. Do not serialize Rust structs directly with
an unstable implementation-specific layout.

Every request carries:

- exact protocol version;
- request ID;
- world UUID and expected source-identity hash;
- optional known world revision;
- priority class;
- cancellation/deadline metadata where applicable.

Every response carries:

- request ID;
- authoritative world revision used for the result;
- source-identity hash;
- result status;
- content hash for each bulk payload.

Required logical messages:

1. `Hello` / `Welcome`: require the exact version, exchange current capabilities, and return the
   manifest.
2. `GetChunkBatch`: request coordinates and priority as one batch.
3. `ChunkBatch`: return snapshots independently so partial completion is useful.
4. `GetSurfaceTileBatch` / `SurfaceTileBatch`.
5. `ApplyEditBatch`: idempotency key, expected revision, and voxel mutations.
6. `EditCommitted`: assigned revision plus accepted mutations.
7. `SubscribeChanges`: resume from a known revision.
8. `CancelRequests`.
9. `Health` / `ServerStats`: development and benchmark diagnostics.

Do not add a general `SampleVoxel` request to the production protocol. If a diagnostic version is useful,
keep it behind a development capability.

### Chunk payload

A returned chunk snapshot should contain:

- the authoritative edited 32-cubed voxel payload;
- a one-voxel meshing halo, or explicit adjacent boundary planes, from the same revision;
- chunk coordinate;
- world revision and identity hash;
- material schema;
- content hash.

The halo prevents synchronous generator callbacks during meshing. Confirm the exact access envelope of
`mesh_chunk` with a test before choosing six planes versus a full 34-cubed block.

Advance the `VXCH` codec rather than embedding the compile-time procedural generator version forever.
The next codec version should validate a manifest/source hash supplied by the session. Keep palette and
bit packing; measure compression before adding a second general-purpose compression layer.

### Surface payload

The service should generate edit-aware far-surface products because the client no longer has the source.
Prefer a renderer-neutral height/material/water grid when its bandwidth is acceptable. A versioned
`SurfaceTileMesh` payload is also allowed because it is currently produced by `world`, not `render`, but
it must not contain GPU handles or backend-specific layout.

## Local transports

### Current browser path

- Bind a binary WebSocket only to `127.0.0.1` on an ephemeral port.
- The launcher/dev task starts `voxels-worldd`, waits for readiness, and passes the endpoint plus a random
  per-launch capability token to the page without hard-coding a port.
- Each WebSocket binary message contains one protocol envelope or one bulk fragment.
- Batch requests and responses; never create one WebSocket message per voxel or column.
- Enforce bounded client queues so a fast camera cannot accumulate obsolete generation.

### Highest-throughput native path

Implement after the semantic protocol is stable:

- length-delimited control frames over a Unix-domain socket;
- the same inline bulk payloads as WebSocket for small results;
- optional immutable shared-memory buffers for large batches, with handles passed over the Unix socket;
- explicit buffer ownership/acknowledgement and generation counters to prevent stale-slot reuse.

Use benchmarks before enabling shared memory. A generated chunk is 65,536 bytes before palette packing,
while diffusion inference is much more expensive than copying that amount over a local socket. Unix
sockets may already make transport cost negligible. Shared memory is justified only if profiles show
copying or codec work on the critical path.

### Future network path

A future QUIC or WebTransport implementation should carry the same logical envelopes. Large independent
chunk responses can use separate streams to avoid head-of-line blocking. This plan does not implement
that transport, but protocol code must not contain socket paths, file descriptors, shared-memory offsets,
or loopback assumptions. Those belong to transport-specific extensions negotiated in `Welcome`.

## Scheduling and performance

`voxels-worldd` should use three bounded execution domains:

1. an async I/O runtime for connections, persistence coordination, and request lifecycle;
2. a fixed CPU pool for procedural composition, codecs, and surface products;
3. one Terrain Diffusion device executor owning the model and Metal command stream.

Do not launch multiple competing Metal model processes. Batch compatible spatial requests in the device
executor, preserve priority between near chunks, visible surface tiles, and speculative prefetch, and put
strict limits on queued work.

Coalesce duplicate requests by `(world identity, revision, product key)`. Cancellation should remove work
that has not started; completed immutable products may still enter the cache. Return independently useful
results as they finish rather than waiting for an entire camera batch.

Recommended priority order:

1. collision-critical/resident near chunks;
2. visible near chunks;
3. visible coarse surface tiles;
4. finer replacement surface tiles;
5. prefetch.

Keep the client's existing ticket revisions and stale-result rejection. Server revisions protect world
state; client ticket revisions protect current render interest. They solve different races.

## Persistence and edits

Move the authoritative edit journal from browser OPFS to native SQLite in `world-service`:

- WAL mode;
- one ordered revision sequence per world;
- idempotent operation IDs;
- transactions for edit batches;
- periodic compact snapshots plus an append-only change log;
- manifest and source identity stored with every world;
- cache indices separate from authoritative edit data.

Keep camera persistence client-local. Split the existing browser `Store`, which currently combines camera
and world edits.

Terrain Diffusion macro tiles are expensive and may not be bit-identical across GPU/framework versions.
Treat accepted generated macro tiles as immutable, content-addressed world data and persist them. The
server is authoritative; multiplayer clients must never independently regenerate and compare floating
point terrain.

The current procedural source can continue regenerating pristine chunks because it has strict integer
determinism. Cache policy may differ by source without changing the protocol.

During early development, do not import or upgrade older browser worlds. A schema or authority change
gets a new namespace and starts empty. Leave old OPFS files untouched so development and tests never
reset or overwrite local data, but do not read them from the current build.

## Terrain Diffusion provider

### Implementation target

Create `world-terrain-diffusion` as a native Rust implementation with:

- `safetensors` weight loading;
- a Rust implementation of `EDMUnet2D`;
- the published coarse, base, and decoder model configurations;
- the EDM/DPM scheduler;
- coordinate-keyed deterministic noise;
- lazy infinite tile assembly and cache semantics;
- conditioning and unit conversion into `MacroBlock`;
- a Candle Metal backend on Apple silicon.

Candle's backend is native Metal rather than PyTorch's `mps` device. That still executes on the Apple GPU
without Python or CUDA and is the preferred Rust-first target. If strict use of MPSGraph is required,
isolate an alternative backend behind the same internal tensor executor and benchmark a small Rust FFI
bridge. Do not couple world-service or protocol code to Candle, MLX, MPSGraph, or Core ML.

Do not assume the framework name determines the fastest implementation. Before completing the full port,
benchmark the coarse model with the same FP32/FP16 inputs using:

1. Candle's native Metal backend;
2. MPSGraph through a minimal Rust/system-framework FFI, if the required operators export cleanly;
3. an ahead-of-time Core ML package behind the same Rust executor, only as an Apple-specific comparison.

Select the production executor from measured model latency, peak unified memory, warm batch throughput,
output agreement, startup cost, and integration complexity. The runtime process and orchestration remain
Rust in every case. No candidate may require Python or Swift as a runtime sidecar.

MLX may be used as a numerical/performance reference, but do not make Python or Swift/MLX a runtime
dependency of the shipped provider.

### Porting sequence

1. Freeze a specific Terrain Diffusion repository commit and model revision.
2. Use its ONNX export and PyTorch implementation only as validation oracles.
3. Port primitive magnitude-preserving layers and compare outputs layer by layer.
4. Port one `EDMUnet2D`; load safetensors and compare a fixed forward pass.
5. Port the scheduler and compare every denoising step.
6. Port the coarse stage and deterministic spatial noise.
7. Port base latent generation and overlap behavior.
8. Port the decoder and tile stitching.
9. Emit versioned macro blocks, then feed the existing voxel composer.
10. Measure FP32 and FP16. Select reduced precision only after seam, range, and repeatability tests pass.

Add a `--require-metal` service option. A world whose manifest requires the Terrain Diffusion Metal
provider must fail clearly at startup if the model, weights, or device cannot load; it must not produce a
different CPU-derived world under the same identity.

### Determinism policy

Exact cross-backend floating-point identity is not a requirement. Canonical authority is provided by:

- immutable source and weight hashes;
- deterministic coordinate-derived inputs;
- content-addressed persisted macro blocks;
- one server choosing the accepted result;
- revisions and hashes on all derived products.

Within one pinned backend and precision, require repeatable outputs in tests. Across backend upgrades,
either prove compatibility within the declared quantization rule or create a new source identity.

## Implementation phases

### Phase 0: baselines and invariants

- Add benchmarks for current chunk generation, halo sampling, codec size/time, surface generation, and
  frame admission.
- Record golden hashes for representative ordinary, road, water, landmark, and Cinder Vault chunks.
- Add a test that records the exact mesher halo coordinates.
- Define latency, throughput, memory, and startup baselines before moving code.

Exit: reproducible baseline artifacts exist and no production behavior has changed.

### Phase 1: in-process source abstraction

- Introduce world/source identity types.
- Wrap the existing generator in the batch-oriented source interfaces while it remains in-process.
- Refactor `EditMap` so it receives generated values or generic samplers rather than a concrete
  copyable `Generator`.
- Refactor surface generation and probes to consume bounded source products.
- Advance the chunk codec identity model.
- Keep golden output hashes identical for `procedural-v16`.

Exit: the game still runs in one worker, but no consumer outside the provider depends on the concrete
generator.

### Phase 2: protocol and local daemon

- Add `world-protocol`, `world-service`, `world-client`, and `worldd`.
- Implement manifest negotiation, chunk batches, cancellation, health, and binary WebSocket transport.
- Run the current procedural provider in `worldd`.
- Keep edits temporarily read-only or mirrored while validating snapshot flow.
- Gate the new path behind a development flag; retain the in-process path for counterproof.

Exit: the browser renders and collides against chunks generated only in the native sidecar, with golden
parity and bounded queues.

### Phase 3: authority and derived products

- Move edits, revisions, native SQLite persistence, and change subscriptions to the service.
- Return edited chunks with meshing halos.
- Move edit-aware surface and water products to the service.
- Split camera persistence from edit persistence.
- Cut browser persistence to a fresh server-authority namespace.
- Remove normal shell calls to `Generator`.

Exit: stopping the daemon stops new world data; no hidden local generator or edit authority remains in
the game.

### Phase 4: harden the local boundary

- Add reconnect/resume from revision, idempotent edit retry, malformed-frame tests, protocol version
  tests, and service crash recovery.
- Add Unix-domain-socket conformance tests using the same messages.
- Benchmark inline Unix-socket bulk transfer; add shared memory only if it materially improves an observed
  bottleneck.
- Keep one exact protocol version and hard-fail every mismatch.

Exit: all transports pass one conformance suite, and restart/reconnect cannot lose committed edits.

### Phase 5: native Terrain Diffusion spike

- Implement and validate the coarse model first.
- Prove Metal execution, fixed-input parity, peak memory, cold start, and warm throughput.
- Continue through base and decoder stages only if the spike meets explicit budgets.
- Persist macro tiles and expose a development Terrain Diffusion world manifest.

Exit: a fixed region can be generated with no Python process, no PyTorch runtime, and no CUDA; GPU use is
verified and outputs survive restart through the native cache.

### Phase 6: selectable Terrain Diffusion worlds

- Complete infinite tile semantics, conditioning, scheduling, and cache eviction.
- Compose canonical voxels and authored content from diffusion macro blocks.
- Add world creation/configuration UI or CLI outside the protocol core.
- Run seam, route-grade, water, biome, cave, edit, and LOD invariants.

Exit: selecting either source changes only the manifest/provider configuration; the game client and wire
messages are unchanged.

## Verification gates

### Correctness

- Existing procedural voxel contents and surface products remain byte-identical through Phase 2; an
  intentionally versioned transport or codec envelope may differ.
- Negative coordinates, chunk boundaries, world coordinate limits, and all material IDs round-trip.
- Chunk core and halo come from one world revision.
- Reverting an edit restores the exact source material.
- Stale client ticket responses and stale world revisions are independently rejected.
- Route, water, cave, feature-pristine, and surface-LOD invariants still pass.

### Performance

- Compare warmed batched service throughput with the current in-process benchmark.
- The process split must not reduce procedural generation throughput by more than 5% without an explained
  tradeoff approved from profiles.
- Streaming must remain within the current frame admission budgets; transport callbacks may not perform
  unbounded decode or mesh work.
- Track p50/p95/p99 request latency, queue wait, generation, edit application, encoding, bytes transferred,
  and stale/cancelled work.
- Track daemon RSS and cache residency separately from browser memory.
- Terrain Diffusion benchmarks must report cold model load, first tile, warm adjacent tile, random tile,
  peak unified memory, cache hit ratio, and Metal device time.

### Failure and version cuts

- Kill and restart `worldd` during generation and during an edit transaction.
- Disconnect/reconnect the browser and resume changes from its last revision.
- Reject source identity, model hash, material schema, and protocol mismatches before accepting payloads.
- Verify every non-current client version is rejected before any world product is exchanged.
- Corrupt cached macro tiles and chunk payloads and prove they are rejected without touching authoritative
  edits.
- Run every persistence test against explicit temporary databases and cache roots.

## Instrumentation

Expose structured tracing and a development stats message with:

- queue depth by priority and product type;
- active/cancelled/coalesced requests;
- cache hit/miss/eviction counts and bytes;
- generation, model, composition, encoding, and transport durations;
- current world revision and connected clients;
- selected provider, source identity hash, device, precision, and whether Metal is active.

Do not use high-cardinality voxel coordinates as default metric labels. They may appear in sampled trace
events.

## First implementation change set

The first client should implement only Phase 0 and Phase 1. It should not create the daemon yet.

Expected first change set:

1. baseline benchmarks and mesher-halo test;
2. `WorldSourceIdentity` and `WorldManifest` in `world`;
3. batch-oriented source-product types;
4. an adapter around the current generator;
5. source-agnostic edit comparison/application APIs;
6. source-aware chunk codec versioning;
7. golden parity tests;
8. a short follow-up note listing the remaining concrete `Generator` dependencies.

This ordering makes the process boundary a mechanical extraction of an already tested interface. It also
provides the same contract the Terrain Diffusion implementation will target, without putting ML concerns
on the critical path of the server split.

## Completion definition

The overall project is complete when:

- the browser game contains no production world generator;
- `voxels-worldd` is the only base-world and edit authority;
- current procedural worlds retain their established outputs and local data;
- source selection is manifest-driven and invisible to the client protocol;
- all local transports share one conformance suite;
- Terrain Diffusion can generate a world region using a pinned, native Rust implementation on the Apple
  GPU, without Python, PyTorch, or CUDA;
- generated diffusion macro data is versioned, hashed, persisted, and safe across restarts;
- the protocol can be placed on a network transport later without redesigning world products, revisions,
  or source identity.

## Research anchors

- [Terrain Diffusion](https://github.com/xandergos/terrain-diffusion) publishes the Python reference,
  infinite-world pipeline, MIT license, and safetensor models.
- Its [ONNX exporter](https://github.com/xandergos/terrain-diffusion/blob/master/terrain_diffusion/onnx/export.py)
  exports and verifies the coarse, base, and decoder `EDMUnet2D` models.
- [Hugging Face Candle](https://github.com/huggingface/candle) is the Rust/Metal precedent and includes
  native Stable Diffusion implementations and safetensor loading.
- [Apple MLX Swift examples](https://github.com/ml-explore/mlx-swift-examples) provide a second native
  Apple-silicon diffusion reference, but are not the intended runtime dependency.
- [Apple's PyTorch Metal documentation](https://developer.apple.com/metal/pytorch/) explains the existing
  MPS backend and is useful for producing a reference performance run before the native Rust port.
