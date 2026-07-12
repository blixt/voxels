# Voxels rewrite architecture

Voxels is a Rust-first, WebGPU-only procedural world. TypeScript owns only browser concerns that Rust
cannot own directly: the DOM canvas, normalized user-input capture, capability checks, worker boot, and
small development hooks. The game simulation, procedural generation, voxel representation, meshing,
rendering, persistence, and binary codecs live in Rust.

## Workspace boundaries

- `core/` is portable and host-testable. It owns commands, player/camera state, physics, exact voxel
  DDA picking, simulation, and game rules. It has no GPU, browser, filesystem, or JavaScript
  dependencies.
- `world/` is portable and host-testable. It owns chunk coordinates/data, materials, procedural
  generation, edit overlays, greedy meshing, and durable voxel codecs.
- `runtime/` is portable and host-testable. It owns deterministic chunk interest, bounded per-frame
  generation/meshing/upload admission, revisioned work tickets, stale-result rejection, eviction
  hysteresis, and streaming diagnostics. It owns no payloads or GPU resources.
- `render/` is a platform-neutral WGPU library. It owns cameras, GPU resources, mesh uploads, shaders,
  culling, frame timing, and all visible UI. Its pure mission-control model produces glass/text draw
  lists; a two-pass WGPU backend presents the sampled world backdrop, refractive chrome, embedded-font
  glyphs, the controls toast, and crosshair. It names no web types so a native shell can reuse it.
- `shell/` is the `wasm32-unknown-unknown` leaf. It owns the `wasm-bindgen` API, the transferred
  `OffscreenCanvas`, worker animation clock, packed input decoding, and OPFS persistence.
- `web/` is the deliberately thin browser harness. Its only visible element is the canvas. Input is
  batched into fixed-size binary records and transferred to the worker; TypeScript only mirrors Rust's
  cursor/capture mode because pointer lock is a browser responsibility. Semantic actions, status, UI
  layout, text, world state, and feature behavior do not cross into TypeScript.

The renderer and simulation run together in one dedicated worker. The shell advances runtime-issued
generation, meshing, and upload tickets under independent frame budgets, while the renderer only owns
per-chunk GPU meshes. That keeps generation, meshing, SQLite, and GPU submission off the browser main
thread without inventing a JavaScript coordination layer. Additional Rust/WASM workers are deferred
until benchmarks show generation or meshing is the frame-time bottleneck.

## World representation

The live world is a sparse map of fixed-size cubic chunks. A chunk stores a compact material id per
voxel and is the unit of generation, editing, persistence, remeshing, culling, and streaming. The
authoritative near field keeps the required 10 cm voxel resolution and uses face-culling plus greedy
rectangle merging. A column becomes render-ready only when all desired vertical chunks are resident,
so a partially streamed stack cannot expose an open terrain slice.

Generator v6 builds one reusable `SurfaceSample` per X/Z column from climate, continental, ridge,
detail, dune, and volcanic fields. Six dominant regional identities—verdant forest, wind-cut moor,
alpine, red badlands, pale dunes, and volcanic terrain—select surface ecology and geology, while their
normalized weights blend height modifiers continuously across boundaries. Canonical generation, LOD
surface summaries, edit overlays, and spawn queries consume that same sample. A reusable
`GeneratedColumn` additionally caches terrain fields and tree intersections for repeated Y sampling,
which keeps richer world logic from multiplying meshing-halo cost.

Water is material 13 in material schema v2 and remains part of the same canonical 10 cm voxel field.
It is renderable and editable but non-collidable and non-occluding. Low continental basins fill only
the cells above terrain through the versioned sea level, never underground cave air. Near greedy
meshes split opaque and translucent quads without duplicating a water face against an opaque bank.
The edit journal stores Water/Air overrides exactly like every other material, so removing a surface
cell and restoring its generated water are durable sparse operations rather than renderer effects.
Continental noise shapes flooded areas into a shelf, slope, and navigable basin instead of a shallow
visual sheet. Portable simulation derives exact player immersion from AABB overlap with those same
water voxels, uses hysteretic swim state and buoyancy while retaining solid collision, and accepts
restored underwater cameras. The shell caches generated columns across fixed-step collision/fluid
queries so the richer probe does not recompute climate fields per voxel.

The frame uniform appends a version-asserted medium vector containing smoothed underwater blend, eye
depth, physical immersion, and local surface height. Opaque and sky shaders apply wavelength-dependent
absorption and in-scattering, terrain caustics, and a world-anchored Snell window; the water shader
handles underside refraction and total internal reflection. Simulation values remain fixed-step and
unsmoothed. Rust/WGPU chrome reports immersion/depth and re-opens a Rust-rendered swim-help toast on
entry. Disabling animated water changes rendering only, never authoritative fluid physics.

Four independently streamable surface rings derive from the same generator and sparse edit overlay at
0.2, 0.4, 0.8, and 1.6 m sampling strides. Each tile covers 32 samples per side and emits a top plus
vertical transition faces down to lower neighbors, including samples across tile boundaries, so
separately generated tiles form a closed shell without cracks. The coarsest ring extends beyond the
220 m fog cutoff; missing finer rings temporarily reveal complete coarse underlays instead of a hole.
All surface meshes remain disposable derivatives: the generator, 10 cm voxels, and sparse edits stay
authoritative.

Each surface level also derives an edit-aware, 2D-greedy water mask at the exact canonical sea Y.
Water patches use the same CPU ownership bounds as terrain, but never inherit the lowered crack-hiding
terrain underlay. Opaque terrain and translucent water use independent GPU arenas. A depth-only water
prepass followed by a premultiplied-alpha color pass gives overlapping wave surfaces deterministic
visibility without letting water cast solid shadows. The shader combines multi-directional procedural
wave normals, Schlick Fresnel reflection, the shared sky/sun environment, distance absorption, HDR
fog, and the same presentation transform as land.

Every terrain surface tile is one opaque-arena allocation partitioned into sixteen contiguous
8x8-cell draw patches; a non-empty water derivative uses the parallel water arena. Each terrain patch
also owns four separately addressable vertical skirts. The renderer submits a skirt only where that
edge touches a different LOD owner, closing height disagreement without paying for internal
same-resolution walls.

Coverage ownership changes at grid-snapped, half-open square boundaries aligned to every participating
patch size. Rust selects whole canonical chunks and surface patches on the CPU, then uses the identical
selection for color and all shadow cascades; no fragment-level discard is needed after a complete
coverage set becomes active. During initial fill, the former continuous shader predicate remains a
safe fallback over complete coarse underlays. A pending focus retains the last active surface set until
all replacement tiles are resident, so movement changes ownership transactionally instead of exposing
partially streamed rings. Trees remain canonical near-field geometry; they are withheld only during the
first incomplete fine-field load so isolated crowns cannot appear before their supporting columns.

Procedural trees have one analytic `SkylineFeature` identity shared by canonical chunk decoration,
random-access voxel sampling, edit invalidation, and surface LOD generation. An anchor-owned surface
patch appends a small stepped trunk/crown cuboid proxy, simplified with distance but using the same
materials and bounds. Any canonical edit that touches the generated feature suppresses its disposable
proxy at every level; an edit outside the analytic tree does not. The canonical boundary snaps to the
96-voxel feature placement grid, keeping a whole tree on one side of the canonical/proxy handoff.

An edit invalidates every surface tile whose sampling footprint depends on that X/Z column. Resident
geometry stays active while its replacement is generated and allocated, then the renderer switches
the mesh and releases the old allocation atomically. Dirty work that leaves the retained streaming
window is discarded because any later load samples the authoritative edit overlay again. Feature
edits additionally invalidate the feature anchor's tile when a crown or branch crosses a tile edge.

Near meshes also bake the established four-level voxel ambient-occlusion term from two side samples
and the diagonal at each face corner. Four 2-bit values participate in the greedy merge key, and the
renderer selects the lower-error triangle diagonal from opposing AO sums. A single 34³ occupancy halo
makes visibility and AO sampling cache-local while preserving authoritative neighbor seams.

Directional sunlight uses three stable cascaded shadow maps in a portable `Depth32Float` texture
array. Practical logarithmic/uniform splits prioritize the editable near field; each frustum slice is
enclosed in a quantized sphere and snapped to its own world-space texel grid to resist shimmer. Shadow
passes reuse greedy mesh allocations, cull each selected slice against its light volume, preserve the
same CPU geometric ownership as the color pass, and apply 3x3 comparison PCF only to direct sunlight.
The Rust mission-control toggle skips all
three caster passes, while live diagnostics expose their draw-call cost.

Outdoor lighting is one Rust-owned environment shared by shadow projection, sky radiance, voxel
lighting, and aerial perspective. The world first renders in linear HDR to `Rgba16Float`; the present
pass applies the Khronos PBR Neutral tone mapper and an sRGB transfer before the Rust UI is composited.
Refractive glass samples and maps that same HDR backdrop before mixing its display-space chrome, so a
panel cannot expose a differently transformed copy of the world. Material base colors are converted
from authored sRGB values, ambient occlusion attenuates indirect light rather than direct sunlight,
and per-material roughness drives a compact dielectric highlight. Exponential height fog converges on
the same horizon/zenith radiance rendered by the procedural sky, which also includes a world-anchored
three-octave cloud layer.

The first persistent chunk format is versioned and little-endian:

1. a small magic/version header and chunk coordinates;
2. a chunk-local palette of stable material ids;
3. palette indices packed at `ceil(log2(palette length))` bits per voxel in Y-Z-X traversal order;
4. an integrity hash and optional block compression around the packed payload.

Palette + bit packing follows the useful part of modern Minecraft's paletted-container design while
remaining independent of NBT and game-specific registries. Fixed chunks are intentionally preferred
over an octree for the mutable near field: they provide predictable addressing, cheap local edits, and
bounded remesh work. Sparse voxel octrees/DAGs remain candidates for immutable far-field or offline
assets, where their topology compression is a better fit.

## Persistence

SQLite runs inside Rust/WASM through the native SQLite C API exposed by `sqlite-wasm-rs`. Its database
lives in Origin Private File System storage through the `sqlite-wasm-vfs` sync-access-handle pool. OPFS
is worker-only; keeping the engine in a dedicated worker therefore satisfies both rendering and storage
constraints.

SQLite stores structured, queryable state: schema version, world identity and generator version,
player state, and sparse voxel overrides. Each override is an idempotent row keyed by world and voxel;
restoring the generated material removes the row. Versioned palette/bit-packed chunk payloads exist for
future snapshot compaction. If profiling shows write amplification or database size becoming a real
constraint, the same codec can move snapshots into append-only region files while SQLite remains the
transactional index. Region files would group a bounded X/Z tile, use a checksummed offset table, and
write payloads in aligned extents; that complexity is not justified before measurements.

Multi-tab access is single-writer without excluding other tabs. A Web Lock elects one worker as the
SQLite/SAH-pool owner; followers proxy typed camera/edit operations over a BroadcastChannel and queue
for ownership when the leader closes. Follower writes pass through one ordered, coalescing Rust outbox,
and follower requests tolerate a complete VFS retry window plus worker startup. Teardown closes SQLite,
pauses the SAH-pool VFS to synchronously release its OPFS handles, and only then resolves the Web Lock;
queued acquisitions are aborted and the BroadcastChannel is closed. A browser stress test exercises
rapid reload and ownership handoff on an isolated origin. The wire includes the seed and generator
version so an incompatible build cannot silently answer another world's request.

Committed sparse edits are also a Rust-to-Rust multi-tab replication unit. A follower applies its edit
optimistically, the elected leader commits it, applies follower-originated commits to its own engine
because BroadcastChannel does not self-echo, and broadcasts the durable override (including row
removal) to every other follower. Each recipient updates `EditMap`, resident canonical data, remesh
tickets, and affected LOD tiles through the same invalidation path as a local edit. Rendering and
collision therefore converge live instead of waiting for a reload.

## Performance policy

- Never allocate or send one JavaScript object per input sample, voxel, face, or chunk.
- Generate and mesh only dirty/resident chunks, with bounded work admitted each frame.
- Mesh chunk boundaries using neighbor samples so hidden seam faces are not emitted.
- Suballocate immutable chunk and surface-ring meshes from separate coalescing opaque/water GPU arena
  pages, replacing only the allocation whose source changes. Pack common patch bodies before optional
  skirts and coalesce adjacent selected ranges into draw spans.
- Frustum-cull chunks on CPU; add occlusion/indirect drawing only after GPU captures justify it.
- Keep deterministic host benchmarks for generation, codec round-trips, meshing, and edit replay.
- Expose lightweight browser frame and residency snapshots for end-to-end regression automation.

## Research basis

- WGPU 30 directly supports an `OffscreenCanvas` surface and the browser WebGPU backend:
  <https://docs.rs/wgpu/30.0.0/wasm32-unknown-unknown/wgpu/enum.SurfaceTarget.html>
- WebGPU depth texture arrays and comparison samplers provide a portable shadow-map path; WGSL's
  `textureSampleCompareLevel` fixes mip level zero and is valid in non-uniform cascade selection:
  <https://gpuweb.github.io/gpuweb/wgsl/#texturesamplecomparelevel-builtin>
- The official WebGPU sample demonstrates depth-only shadow-map rendering and comparison sampling:
  <https://webgpu.github.io/webgpu-samples/?sample=shadowMapping>
- SQLite recommends `opfs-sahpool` when performance matters more than concurrent connections and notes
  that OPFS APIs are worker-only:
  <https://sqlite.org/wasm/doc/tip/persistence.md>
- Web Locks are origin-scoped, available in workers, held for the lifetime of the callback promise, and
  support aborting queued acquisition:
  <https://www.w3.org/TR/web-locks/>
- A synchronous OPFS access handle owns an exclusive file lock until it is closed:
  <https://fs.spec.whatwg.org/#api-filesystemsyncaccesshandle-close>
- The Rust SQLite bindings and OPFS VFS expose the SQLite C API and sync-access-handle pool directly to
  `wasm32-unknown-unknown`:
  <https://docs.rs/sqlite-wasm-rs/0.5.5/sqlite_wasm_rs/>
  <https://docs.rs/sqlite-wasm-vfs/0.2.0/sqlite_wasm_vfs/>
- Greedy meshing reduces exposed voxel faces into larger rectangles while keeping chunk-local rebuilds:
  <https://0fps.net/2012/06/30/meshing-in-a-minecraft-game/>
- Minecraft's paletted chunk sections establish a practical precedent for local palettes plus packed
  indices and compression-friendly traversal:
  <https://minecraft.wiki/w/Java_Edition_protocol/Chunk_format>
- Sparse voxel octrees are valuable for sparse, mostly static data but retain ordinary leaf blocks for
  much of the content:
  <https://research.nvidia.com/sites/default/files/pubs/2010-02_Efficient-Sparse-Voxel/laine2010tr1_paper.pdf>
- Geometry/attribute DAG work reinforces separating compressed topology from palette-based attributes:
  <https://doi.org/10.1111/cgf.12841>
- Khronos PBR Neutral provides a bounded, hue-preserving reference display transform for linear HDR
  PBR output:
  <https://github.com/KhronosGroup/ToneMapping/tree/main/PBR_Neutral>
- GPU Gems derives real-time water from summed directional waves, analytic surface derivatives, and
  reflection/refraction rather than treating the surface as a flat alpha overlay:
  <https://developer.nvidia.com/gpugems/gpugems/part-i-natural-effects/chapter-1-effective-water-simulation-physical-models>
- WebGPU defines the depth-stencil state used by the opaque water color pass:
  <https://gpuweb.github.io/gpuweb/#dictdef-gpudepthstencilstate>

## Deliberate non-goals for the conversion

- Preserving old gameplay systems or exact visual output.
- Supporting WebGL or browsers without WebGPU and transferable `OffscreenCanvas`.
- Treating generated terrain as canonical saved data when a seed plus sparse edits is sufficient.
- Introducing ECS, networking, octrees, bindless rendering, or GPU-driven meshing without benchmark
  evidence.
