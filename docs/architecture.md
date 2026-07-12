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

Generator v4 builds one reusable `SurfaceSample` per X/Z column from climate, continental, ridge,
detail, dune, and volcanic fields. Six dominant regional identities—verdant forest, wind-cut moor,
alpine, red badlands, pale dunes, and volcanic terrain—select surface ecology and geology, while their
normalized weights blend height modifiers continuously across boundaries. Canonical generation, LOD
surface summaries, edit overlays, and spawn queries consume that same sample. A reusable
`GeneratedColumn` additionally caches terrain fields and tree intersections for repeated Y sampling,
which keeps richer world logic from multiplying meshing-halo cost.

Four independently streamable surface rings derive from the same generator and sparse edit overlay at
0.2, 0.4, 0.8, and 1.6 m sampling strides. Each tile covers 32 samples per side and emits a top plus
vertical transition faces down to lower neighbors, including samples across tile boundaries, so
separately generated tiles form a closed shell without cracks. The coarsest ring extends beyond the
220 m fog cutoff; missing finer rings temporarily reveal complete coarse underlays instead of a hole.
All surface meshes remain disposable derivatives: the generator, 10 cm voxels, and sparse edits stay
authoritative.

Every surface tile is one arena allocation partitioned into sixteen contiguous 8x8-cell draw patches.
Each patch also owns four separately addressable vertical skirts. The renderer submits a skirt only
where that edge touches a different LOD owner, closing height disagreement without paying for internal
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
for ownership when the leader closes. Follower requests retry across short handoff gaps, while SAH-pool
installation retries through stale sync-access handles left briefly by rapid reloads. The wire includes
the seed and generator version so an incompatible build cannot silently answer another world's request.

## Performance policy

- Never allocate or send one JavaScript object per input sample, voxel, face, or chunk.
- Generate and mesh only dirty/resident chunks, with bounded work admitted each frame.
- Mesh chunk boundaries using neighbor samples so hidden seam faces are not emitted.
- Suballocate immutable chunk and surface-ring meshes from coalescing GPU arena pages, replacing only
  the allocation whose source changes. Keep LOD patches as byte ranges within each tile allocation and
  coalesce adjacent selected ranges into draw spans.
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

## Deliberate non-goals for the conversion

- Preserving old gameplay systems or exact visual output.
- Supporting WebGL or browsers without WebGPU and transferable `OffscreenCanvas`.
- Treating generated terrain as canonical saved data when a seed plus sparse edits is sufficient.
- Introducing ECS, networking, octrees, bindless rendering, or GPU-driven meshing without benchmark
  evidence.
