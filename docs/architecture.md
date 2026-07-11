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
  culling, frame timing, and rendering. It names no web types so a future native shell can reuse it.
- `shell/` is the `wasm32-unknown-unknown` leaf. It owns the `wasm-bindgen` API, the transferred
  `OffscreenCanvas`, worker animation clock, packed input decoding, and OPFS persistence.
- `web/` is the deliberately thin browser harness. Input is batched into fixed-size binary records and
  transferred to the worker; semantic actions and world state do not cross into TypeScript.

The renderer and simulation run together in one dedicated worker. The shell advances runtime-issued
generation, meshing, and upload tickets under independent frame budgets, while the renderer only owns
per-chunk GPU meshes. That keeps generation, meshing, SQLite, and GPU submission off the browser main
thread without inventing a JavaScript coordination layer. Additional Rust/WASM workers are deferred
until benchmarks show generation or meshing is the frame-time bottleneck.

## World representation

The live world is a sparse map of fixed-size cubic chunks. A chunk stores a compact material id per
voxel and is the unit of generation, editing, persistence, remeshing, culling, and streaming. Near-field
chunks use face-culling plus greedy rectangle merging; distant detail can later use coarser derived
chunks without changing the canonical data.

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

Multi-tab access is single-writer. The first implementation reports contention clearly; leader election
and follower proxying can be added without changing `core` or `render`.

## Performance policy

- Never allocate or send one JavaScript object per input sample, voxel, face, or chunk.
- Generate and mesh only dirty/resident chunks, with bounded work admitted each frame.
- Mesh chunk boundaries using neighbor samples so hidden seam faces are not emitted.
- Upload immutable mesh buffers once and reuse them until that chunk changes.
- Frustum-cull chunks on CPU; add occlusion/indirect drawing only after GPU captures justify it.
- Keep deterministic host benchmarks for generation, codec round-trips, meshing, and edit replay.
- Expose lightweight browser frame and residency snapshots for end-to-end regression automation.

## Research basis

- WGPU 30 directly supports an `OffscreenCanvas` surface and the browser WebGPU backend:
  <https://docs.rs/wgpu/30.0.0/wasm32-unknown-unknown/wgpu/enum.SurfaceTarget.html>
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

## Deliberate non-goals for the conversion

- Preserving old gameplay systems or exact visual output.
- Supporting WebGL or browsers without WebGPU and transferable `OffscreenCanvas`.
- Treating generated terrain as canonical saved data when a seed plus sparse edits is sufficient.
- Introducing ECS, networking, octrees, bindless rendering, or GPU-driven meshing without benchmark
  evidence.
