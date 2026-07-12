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

The current generator builds one reusable `SurfaceSample` per X/Z column from climate, continental, ridge,
detail, dune, and volcanic fields. Six dominant regional identities—verdant forest, wind-cut moor,
alpine, red badlands, pale dunes, and volcanic terrain—select surface ecology and geology, while their
normalized weights blend height modifiers continuously across boundaries. Canonical generation, LOD
surface summaries, edit overlays, and spawn queries consume that same sample. A reusable
`GeneratedColumn` additionally caches terrain fields and landmark intersections for repeated Y sampling,
which keeps richer world logic from multiplying meshing-halo cost.

Water remains material 13 in material schema v3 and part of the same canonical 10 cm voxel field.
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
partially streamed rings. Landmarks remain canonical near-field geometry; they are withheld only
during the first incomplete fine-field load so isolated upper geometry cannot appear before its
supporting columns.

Each region owns an analytic `SkylineFeature` archetype shared by canonical chunk decoration,
random-access voxel sampling, edit invalidation, and surface LOD generation: broadleaf, limestone tor,
alpine needle, clay hoodoo, oriented dune arch, or basalt-column cluster. All are ordinary canonical
10 cm voxels rather than renderer entities, so picking, collision, removal, restoration, codecs, and
sparse persistence require no special case. An anchor-owned surface patch appends at most four cuboid
proxies (24 quads) using the same stable materials and conservative bounds, adding no per-landmark draw
object or allocation. Any canonical edit that touches generated feature material suppresses its
disposable proxy at every level; reverting the edit restores it. The canonical boundary snaps to the
96-voxel feature placement grid, keeping a whole landmark on one side of the canonical/proxy handoff.

Independent placement probability made the landmark catalog read as uniform scatter even when each
archetype was distinct. Generator v8 adds a deterministic 8x8-feature-cell (76.8 m) composition layer
above that safe ownership unit. Each macro cell selects a cluster, ring, clearing, or locally oriented
procession grammar; its pure influence field modulates the existing regional probability and identifies
one hero candidate. Background, companion, and hero prominence increase height by 0%, 25%, and 50% and
expand canonical/proxy radii within the existing ten-voxel conservative bound. The representation still
returns at most one feature per 96-voxel cell, so stable identity, fixed column storage, 24-quad proxy
caps, editing, and LOD handoff remain unchanged. A fixed-seed checksum covers placement, archetype,
anchor, height, orientation, variant, and prominence across positive and negative cells.

Generator v9 adds the First Pilgrim Road as a versioned six-node polyline in canonical 10 cm
coordinates. A single projection returns the nearest segment, tangent, signed lateral offset,
cumulative distance, core coverage, and shoulder blend. The column generator first produces natural
terrain and then applies that route exactly once, so canonical chunks, collision, edits, water,
surface LOD, and random-access sampling cannot disagree. The 3.6 m bed is fully graded while a smooth
shoulder reaches 5.4 m from center; host invariants limit longitudinal steps to the player's 30 cm
step height and preserve a dry cardinal-connected sampled core at all four LOD strides.

Generator v10 gives the route five stable `(route id, ordinal)` landmarks at 28.8 m cadence. Route
cells take precedence over ambient placement and alternate sides while cycling editable cairns,
waystones, and ruined arches. Their canonical analytic shapes use ordinary material voxels, and their
bounded LOD proxies use the same pristine-edit suppression and anchor ownership as regional
landmarks. The final segment deliberately retains a nearby badlands hoodoo as a destination
silhouette. The Rust Mission Control context menu reconstructs these identities to tour the road;
no route, teleport, or UI semantics cross into TypeScript.

Generator v11 replaces the composition layer's generic 50%-taller heroes with six append-only
semantic identities: elder canopy, tor circle, needle gate, buried ribs, buried colonnade, and basalt
crown. Each is an analytic ordinary-voxel form with an exact kind-specific bound. The one hero cell in
each 76.8 m composition area may use an 18-voxel radius, while established background, companion, and
route identities retain their previous anchors and smaller bounds. Disposable LOD representations use
at most four boxes (24 quads), stay anchor-owned, and disappear when any canonical feature voxel is
edited; restoring the generated material removes the sparse override and recreates the exact proxy.
Ordinary candidates reject water before computing composition identity, so rare hero work does not
inflate every column's hot path. The SQLite world record advances generator identity without dropping
existing sparse edits.

Generator v12 extends the First Pilgrim Road from 164.7 m to 753.0 m with 42 frozen canonical nodes.
A deterministic terrain-aware Rust search crosses verdant forest, wind moor, volcanic terrain, and
alpine country before terminating beside a Needle Gate. Exact 10 cm replay proves a dry player-width
bed, no more than 30 cm cut or 20 cm fill, a maximum authored grade below 12%, and cardinal continuity
on every surface-LOD lattice. The same cadence now yields 26 stable editable route landmarks. Their
ordinal catalog and feature-cell lookup are immutable derived indices, so ambient feature generation
does not rebuild or linearly scan every station.

Generator v13 gives the atlas-defined Cinder Vault a final analytic CSG pass: seven ellipsoidal
nodes and six anisotropic conduits form one dry, enterable route with a sealed Basalt shell. The pass
runs after ambient features in chunk and cached-column generation, so unrelated procedural content
cannot puncture or fill the authored void. Deep edits invalidate canonical chunks without rebuilding
heightfield LODs. A portable nine-ray enclosure probe reads edited resident chunks first, then falls back to
canonical generation; Rust/WGPU uses the result for asymmetric eye adaptation, cave air,
outdoor-light rejection, and an optional bounded headlamp.

Generator v14 adds four supported crystal spires using append-only material 14. They remain ordinary
opaque 10 cm voxels for collision, greedy meshing, editing, SQLite persistence, and palette-codec
round trips; material schema v3 and the derived material atlas grow from 14 to 15 stable layers.

Generator v15 reserves one append-only skyline identity for the Cinder Vault mouth. Four canonical
Basalt volumes form two tapered sentinels around the diagonal entrance, and the identical 24-quad
positive proxy is emitted at every surface LOD. This keeps the coarse heightfield watertight while
making the entrance readable outside the 9.6 m canonical ring. Anchor ownership, patch packing, and
the existing pristine-feature test ensure the tell appears once; editing any canonical sentinel voxel
suppresses its derived proxy at all levels, and restoring the generated material recreates it exactly.
The protected entrance itself remains canonical Air and is never approximated as a negative LOD hole.

Generator v16 expands the conservative Elder Canopy and Needle Gate bounds to contain their complete
analytic forms. Canonical chunk/region generation, random-access sampling, edit invalidation, and LOD
proxy ownership therefore agree on every voxel those heroes can produce.

Atlas schema v1 adds append-only Rust discriminants for six destinations and five route chapters;
schema v2 appends the Cinder Vault cave-system identity and its authored atlas definition.
Each destination freezes its canonical X/Z and route station, with tests reconstructing the authored
point to within one 10 cm voxel. The shell projects the camera onto the same route index, chooses the
chapter, and forwards only that Rust state to the renderer. Mission Control draws the chapter and
whole-route percentage into its WGPU header; the browser receives no place names or UI model.

Authored routes are compiled once into immutable Rust segment records containing squared length,
normalized tangent, cumulative distance, and a shoulder-expanded corridor bound. Short routes retain
an exact linear candidate order; longer routes compile into deterministic sparse 25.6 m bins with
ascending segment memberships. The global route bound rejects unrelated columns before touching the
index, while occupied-bin queries still apply the exact corridor test before projection or square
root. Sampling preserves the earlier nearest-segment/tie behavior exactly, cumulative-distance queries
use binary partitioning over stored lengths, and station reconstruction retains identical rounded
anchors. Segment identity is `u16`, decoupling route length from per-column arithmetic.

An edit invalidates every surface tile whose sampling footprint depends on that X/Z column. Resident
geometry stays active while its replacement is generated and allocated, then the renderer switches
the mesh and releases the old allocation atomically. Dirty work that leaves the retained streaming
window is discarded because any later load samples the authoritative edit overlay again. Feature
edits additionally invalidate the feature anchor's tile when generated geometry crosses a tile edge.
Retained tiles just outside active coverage still record affected revisions without spending immediate
remesh work. If focus returns before eviction, revision comparison marks the cached mesh stale and
blocks coverage activation until its replacement is uploaded; this prevents multi-tab edits from
reviving an old retained LOD mesh.

Near meshes also bake the established four-level voxel ambient-occlusion term from two side samples
and the diagonal at each face corner. Four 2-bit values participate in the greedy merge key, and the
renderer selects the lower-error triangle diagonal from opposing AO sums. A single 34³ occupancy halo
makes visibility and AO sampling cache-local while preserving authoritative neighbor seams.

Screen-space contact AO complements that stable voxel term with sub-voxel-scale horizon occlusion.
When enabled, the renderer first replays the selected opaque draw list into the sampled
`Depth32Float` scene depth. While geometric LOD ownership is unsettled the depth fragment entry point
uses the same ownership rejection as the color pass; after coverage settles, a fragmentless pipeline
keeps the prepass cheap. A half-resolution `Rg16Float` pass reconstructs view positions and geometric
normals from depth, samples four rotated horizon slices, and stores visibility with view depth. A
depth-aware 3x3 denoise preserves discontinuities, and the opaque color pass performs a bilateral
upsample while loading the exact prepass depth. The combined AO attenuates only sky and ground bounce,
never direct sunlight or dielectric highlights. There is deliberately no temporal history, so edits,
teleports, and LOD replacement cannot leave ghost occlusion. The two half-resolution targets occupy
1,843,200 bytes at 1280x720 and remain resident across the Rust Mission Control A/B toggle.

Directional sunlight uses three stable cascaded shadow maps in a portable `Depth32Float` texture
array. Practical logarithmic/uniform splits prioritize the editable near field; each frustum slice is
enclosed in a quantized sphere and snapped to its own world-space texel grid to resist shimmer. Shadow
passes reuse greedy mesh allocations, cull each selected slice against its light volume, preserve the
same CPU geometric ownership as the color pass, and apply 3x3 comparison PCF only to direct sunlight.
The Rust mission-control toggle skips all
three caster passes, while live diagnostics expose their draw-call cost.

Outdoor lighting is a continuously sampled Rust-owned environment shared by shadow projection, sky
radiance, voxel lighting, cloud shadowing, and aerial perspective. The generator maps the same smooth
regional weights used by terrain into normalized humidity, coldness, aerosol, cloudiness, horizon
warmth, and haze fields. The shell samples those fields only at the camera—never in hot canonical or
LOD column loops—and the renderer eases toward the target independently of display rate. Four
Rust-owned phases (dawn, clear day, golden hour, and blue hour) change sun direction/radiance, fog,
cloud cover, and stars without changing world geometry or adding browser UI state.

The world first renders in linear HDR to `Rgba16Float`; the present pass applies the Khronos PBR
Neutral tone mapper and an sRGB transfer before the Rust UI is composited.
Refractive glass samples and maps that same HDR backdrop before mixing its display-space chrome, so a
panel cannot expose a differently transformed copy of the world. Material base colors are converted
from authored sRGB values, ambient occlusion attenuates indirect light rather than direct sunlight,
and per-material roughness drives a compact dielectric highlight. Exponential height fog converges on
the same horizon/zenith radiance rendered by the procedural sky, which also includes a world-anchored
three-octave cloud layer.

Opaque materials use two deterministic Rust-generated `128x128x15` texture arrays with eight mip
levels through `1x1`: sRGB albedo and unorm averaged tangent normal plus roughness. Stable material ids
index array layers directly, including valid diagnostic Air and Water layers, while water rendering
continues on its dedicated physical shader path. The complete derived atlas is 2,621,400 bytes and is
regenerated at startup rather than persisted.

The atlas sampler uses nearest-neighbor magnification, minification, and mip selection with
anisotropy disabled. This preserves explicit pixel cells on 10 cm voxel faces instead of blending
neighboring texels and mip levels. The box-filtered mip chain remains in place so distant terrain does
not regress to a shimmering level-zero sample.

Axis-aligned face bases derive planar coordinates from world position, so greedy quads never stretch
detail and adjacent canonical chunks, surface LODs, skirts, and landmark proxies sample the same
coordinates without new vertex attributes. Before atlas lookup, those coordinates snap to three
world-aligned cells per canonical voxel axis: every face therefore exposes exactly 3x3 blocks at
3.33 cm spacing, including faces embedded in a larger greedy quad. Explicit texture gradients come
from the unsnapped coordinates, retaining stable mip selection despite the piecewise-constant lookup.
Albedo mips average in linear space before sRGB encoding.
Normal mips retain the length of the averaged vector; the shader converts shortening into additional
roughness before normalizing, suppressing distant specular sparkle. Detail normals affect lighting
only—geometric normals remain authoritative for shadow bias, LOD ownership, targeting, and collision.
Two specialized voxel pipelines let the Rust Mission Control toggle choose a real zero-sample flat
path without expanding the shared frame uniform or adding a per-fragment feature branch.

Emissive materials also derive bounded local-light candidates during greedy meshing. Only exposed
emissive voxels participate; deterministic material-separated `8x8x8` voxel bins retain a half-voxel
centroid sum and count, avoiding both buried lights and a false centroid between disconnected
formations. These records are disposable mesh data, not durable world state. A chunk's candidate list
changes only after its replacement GPU upload succeeds and is removed with the active canonical mesh,
so geometry and lighting cannot disagree during remesh retries or eviction.

The renderer ranks resident candidates by expected camera influence with a stable tie break, tests at
most the strongest 32 against an exact 10 cm camera-to-emitter DDA, and writes at most 16 visible
lights to a fixed 528-byte uniform at frame binding 6. The shell answers visibility from sparse edits,
resident canonical chunks, and generated fallback columns; non-emissive opaque voxels block the
segment while crystals in the same formation do not self-occlude. Each 32-byte light stores world-space
position/radius and linear radiance/intensity; compile-time Rust layout assertions and WGSL's uniform
[address-space layout rules](https://www.w3.org/TR/WGSL/#address-space-layout-constraints) keep the
host and shader representations identical. The fragment path uses a finite-radius smooth window over
inverse-square attenuation following [Filament's punctual-light model](https://google.github.io/filament/Filament.md.html#lighting/directlighting/punctuallights/attenuationfunction).
Crystal self-emission remains separate, and the light contribution is accumulated before cave fog and
exposure. Mission Control can disable the pass without removing candidates, while diagnostics report
resident candidates, active lights, and budget clipping. This bounded forward path is intentionally
the first tier; logarithmic-Z [clustered forward shading](https://diglib.eg.org/items/6342d4d6-5220-4376-a5c6-a153058f4a3c)
remains the promotion seam if dense-emitter stress tests outgrow the fixed budget.

Camera-to-source visibility prevents lights in disconnected caves from shining through intervening
rock, but the current punctual lights remain unshadowed at the receiver. Thin-wall leakage inside a
source's finite radius and one visibility cell is therefore still a known limitation; portal/geodesic
gating handles authored chokepoints but does not pretend this bounded path is full voxel global
illumination.

Portable cave topology starts with an allocation-free visibility graph capped at 16 cells and 32
undirected portals. Portal openness is a 32-bit derived mask, and fixed-array Dijkstra queries return
deterministic geodesic distance with stable cell-index tie breaking. Invalid endpoints, weights, and
oversized definitions are rejected at construction. The graph persists nothing and names no cave;
Cinder Vault topology v1 supplies one exterior cell, seven nearest-node interior cells, a surface-mouth
portal, and the six authored tunnel edges. Every portal owns a deterministic `5x5` canonical probe
plane and remains open while at least four lanes are clear. The mouth uses an X/Z plane across the
surface aperture; interior probes use tunnel-tangent-perpendicular and vertical axes. Sparse edits can
identify affected portals with a seven-bit mask, so unrelated edits require no topology work.

The WASM shell initializes portal state from the hydrated sparse edit map plus generator, then marks
only probe planes touched by local or remote durable edits. Dirty portals recompute before the next
light selection and increment a topology revision only when openness changes. For lights in the same
cell, exact camera-to-source DDA remains authoritative. Across cells, the open graph's geodesic must fit
inside the light's finite selection range; disconnected or overly long routes are reported separately
from rock-occluded and budget-clipped candidates. This permits legitimate around-corner tunnel light
without allowing a nearby Euclidean source to cross the cave shell.
Mission Control renders active/candidate lights, rock and path rejections, open portal count, and
topology revision in its WGPU statistics card; these values are not a browser overlay.

The portal probe voxel API is canonical world data rather than a browser fixture. The isolated
`profile:portal-edits` harness asks the Rust worker to place or revert the 25 mouth-plane overrides;
the browser layer only opens an observer tab, requests snapshots, and reloads. Both tabs derive the
same mask from ordinary remote edits, and a fresh worker reconstructs it from SQLite/OPFS without a
separate persisted topology cache.

Portal-directed streaming remains secondary to the proven radial cylinder. Near Cinder Vault, the
world crate derives conservative chunk AABBs (including a meshing halo) for every reachable node and
open edge, sorts them by camera-relative priority, and exposes a fixed 192-coordinate plan with an
explicit overflow bit. Far from the authored bounds the plan is empty. The runtime normalizes this
interest deterministically, admits it only after primary coverage, and keeps primary plus secondary
metadata under the existing 320-chunk ceiling. Requested, normalized, desired, truncated, active,
and unreachable-active counts make both capacity layers observable.

Fine meshes carry independent radial and portal activation bits. Radial columns activate only after
their complete vertical coverage is resident; portal coordinates activate atomically by X/Z plan
column. Closing a portal removes only portal ownership, so an overlapping radial mesh stays visible,
while unreachable retained allocations stop drawing immediately without requiring a GPU re-upload.
Replacement uploads preserve the reason mask. Retained resident chunks edited outside the current
desired set are still allowed to finish remeshing, preventing a permanently queued transaction.

Placement material is Rust state, selected through the canvas context menu and displayed in the Rust
header. The browser still transmits only raw input records; right-click uses the selected material and the
ordinary sparse-edit/SQLite/remesh path, so GlowCrystal is not a renderer-only decoration.

The first persistent chunk format is versioned and little-endian:

1. a small magic/version header and chunk coordinates;
2. a chunk-local palette of stable material ids;
3. palette indices packed at `ceil(log2(palette length))` bits per voxel in Y-Z-X traversal order;
4. an integrity hash over the decoded voxel materials.

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
write payloads in aligned extents. Optional block compression can be evaluated around those packed
payloads then; neither region wrapping nor compression is part of VXCH v1.

Multi-tab access is single-writer without excluding other tabs. A Web Lock elects one worker as the
SQLite/SAH-pool owner; followers proxy typed camera/edit operations over a BroadcastChannel and queue
for ownership when the leader closes. Follower writes pass through one ordered, coalescing Rust outbox,
and follower requests tolerate a complete VFS retry window plus worker startup. Teardown closes SQLite,
pauses the SAH-pool VFS to synchronously release its OPFS handles, and only then resolves the Web Lock;
queued acquisitions are aborted and the BroadcastChannel is closed. A failed VFS acquisition is retained
as recovery state rather than logged as an engine failure: the worker releases the Web Lock, re-elects,
and reports the retained cause only if the complete request window is exhausted. Browser gates exercise
solo refresh bursts, concurrent multi-tab refreshes, ownership handoff, and a forced 20-attempt stale
lease before successful reacquisition. The wire includes the seed and generator version so an
incompatible build cannot silently answer another world's request.

Committed sparse edits are also a Rust-to-Rust multi-tab replication unit. A follower applies its edit
optimistically, the elected leader commits it, applies follower-originated commits to its own engine
because BroadcastChannel does not self-echo, and broadcasts the durable override (including row
removal) to every other follower. Each recipient updates `EditMap`, resident canonical data, remesh
tickets, and affected LOD tiles through the same invalidation path as a local edit. Rendering and
collision therefore converge live instead of waiting for a reload.

Every local edit also captures the post-invalidation revision of each desired canonical chunk and a
monotonic revision for every affected active or pending surface tile. The shell considers the edit
converged only when those canonical chunks and LOD replacements are resident and the renderer confirms
that the replacement frame reached WGPU submission. Focus eviction explicitly waives representations
that are no longer visible; a later load samples the authoritative edit map again. This makes edit
latency a user-visible, revision-backed measurement rather than a queue-length approximation.

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
- Activision's practical GTAO formulation derives efficient horizon-based visibility and spatial
  denoising suitable for a screen-space implementation:
  <https://www.activision.com/cdn/research/Practical_Real_Time_Strategies_for_Accurate_Indirect_Occlusion_NEW%20VERSION_COLOR.pdf>
- AMD CACAO and Intel XeGTAO provide modern production references for half-resolution operation,
  depth-aware filtering, and implementation validation:
  <https://gpuopen.com/fidelityfx-cacao/>
  <https://github.com/GameTechDev/XeGTAO>
- SQLite recommends `opfs-sahpool` when performance matters more than concurrent connections and notes
  that OPFS APIs are worker-only:
  <https://sqlite.org/wasm/doc/tip/persistence.md>
- Web Locks are origin-scoped, available in workers, held for the lifetime of the callback promise, and
  support aborting queued acquisition:
  <https://www.w3.org/TR/web-locks/>
- A synchronous OPFS access handle owns an exclusive file lock until it is closed:
  <https://fs.spec.whatwg.org/#api-filesystemsyncaccesshandle-close>
- Bridson's bounded Poisson-disk sampler is a useful blue-noise baseline for avoiding uniform-grid
  repetition; the composition director deliberately adds hierarchy and empty space above that local
  spacing problem:
  <https://www.cs.ubc.ca/~rbridson/docs/bridson-siggraph07-poissondisk.pdf>
- Deussen et al. model plant ecosystems as interacting spatial distributions rather than independent
  props, motivating region-scale composition instead of raising one global landmark probability:
  <https://doi.org/10.1145/280814.280898>
- Galin et al. represent procedural roads as paths whose terrain-aware geometry, grading, and
  roadside placement derive from one route rather than unrelated raster effects. The pilgrim road
  adopts that single-authority principle while keeping a deliberately small host-testable polyline:
  <https://perso.liris.cnrs.fr/eric.galin/Articles/2010-roads.pdf>
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
- WebGPU exposes optional pass-boundary timestamp queries; the renderer requests them only when the
  adapter advertises support and resolves results through a non-blocking readback ring:
  <https://gpuweb.github.io/gpuweb/#timestamp>
- WGSL texture sampling uses fragment derivatives for implicit mip selection and supports homogeneous
  2D texture arrays addressed by a separate array index:
  <https://gpuweb.github.io/gpuweb/wgsl/#texturesample-builtin>
- WebGPU defines repeated, filtered, mipmapped samplers and clamps requested anisotropy to the device's
  supported maximum:
  <https://gpuweb.github.io/gpuweb/#sampler-creation>
- Toksvig's normal-map mip filtering uses averaged-normal shortening as a measure of unresolved normal
  variation instead of renormalizing away the information:
  <https://doi.org/10.1080/2151237X.2005.10129203>
- Frostbite's sky/atmosphere/cloud course treats sky radiance, aerial perspective, cloud lighting,
  and time of day as one coherent dynamic system rather than independent color overlays:
  <https://www.ea.com/news/physically-based-sky-atmosphere-and-cloud-rendering>
- Hillaire's production atmosphere technique provides the next physically based upgrade path when the
  current analytic model is no longer sufficient for ground-to-space views:
  <https://sebh.github.io/publications/egsr2020.pdf>
- Bruneton's tested reference implementation documents precomputed multiple scattering, spectral to
  luminance conversion, ozone, and configurable aerosol density profiles:
  <https://ebruneton.github.io/precomputed_atmospheric_scattering/>
- WGSL requires derivative built-ins such as `fwidth` to execute in uniform control flow; the planar
  cloud layer therefore uses analytic edge softness inside its view-dependent branch:
  <https://www.w3.org/TR/WGSL/#derivatives>

## Deliberate non-goals for the conversion

- Preserving old gameplay systems or exact visual output.
- Supporting WebGL or browsers without WebGPU and transferable `OffscreenCanvas`.
- Treating generated terrain as canonical saved data when a seed plus sparse edits is sufficient.
- Introducing ECS, networking, octrees, bindless rendering, or GPU-driven meshing without benchmark
  evidence.
