# Maintenance follow-ups

This is the small running list of repository improvements that have concrete evidence but are not
safe to land as incidental cleanup. An item leaves this list only after its decision or measurement
gate is satisfied.

## Recreate a lost WebGPU surface

`Renderer::render` currently handles `CurrentSurfaceTexture::Lost` and `Outdated` identically by
configuring the existing `Surface`. In wgpu 30, `Outdated` calls for reconfiguration, while `Lost`
requires a new `Surface` from `Instance::create_surface` before configuration. The renderer consumes
its `SurfaceTarget` during construction and retains neither the target nor a host callback that can
create a replacement, so the correct recovery is larger than a local match-arm change.

Define surface ownership across `shell` and `render`, then add deterministic fault injection that
proves:

- `Outdated` reconfigures the existing surface and skips only the affected frame;
- `Lost` creates and configures a new surface without rebuilding unrelated GPU resources;
- device loss escalates to complete device/resource reconstruction instead of retrying the surface.

Reference: [wgpu 30 `CurrentSurfaceTexture`](https://docs.rs/wgpu/30.0.0/wgpu/enum.CurrentSurfaceTexture.html).

## Reuse procedural chunk generation for meshing halos

`ProceduralWorldSource::chunk_with_halo` currently calls `Generator::generate_chunk`, then builds a
34-by-34 `GeneratedRegion` for the halo. The region recomputes the 32-by-32 core column profiles and
feature descriptors that chunk generation just evaluated.

The combined path should construct the authoritative region once and derive both the core `Chunk`
and `MeshingHalo` from it. Before replacing the current path, require:

- material-for-material equivalence across representative, negative-boundary, feature, cave, and
  water chunks;
- identical encoded `ChunkSnapshot` products;
- a Criterion comparison of the production `ChunkWithHalo` request, including allocation counts or
  peak resident memory when available.

Relevant code: `world/src/source.rs` (`chunk_with_halo`) and `world/src/generation.rs`
(`generate_chunk`, `GeneratedRegion`).

## Bound world-response memory end to end

`max_queued_outbound_bytes_per_client` is acquired in `GenerationFrameDelivery::send`, after a
generation task has assembled and compressed its complete response. A slow client can therefore
retain up to its negotiated 16 in-flight responses outside the advertised 32 MiB outbound bound;
each response may approach the separate 16 MiB frame limit. Multiplying that gap across the public
128-client limit is not compatible with treating the outbound semaphore as a complete process-memory
bound.

The response size is not exact until compression finishes, so choose an explicit ownership model
before changing the scheduler: either reserve a conservative maximum before assembly and refund the
difference, or add a process-wide assembly budget that transfers the exact retained-byte permit to
the outbound frame.

Two adjacent queues need to share that ownership model. `FrameReassembler` allows 32 concurrent
16 MiB transfers without an aggregate retained-byte cap, so a peer can make a client retain roughly
512 MiB of partial frames. On the server, `write_frames` drains the bounded outbound channel into
unbounded per-priority `VecDeque`s while traffic permits are pending. Generated products carry byte
permits, but direct edit and control frames do not, so a slow socket can bypass both the outbound-byte
limit and the documented edit subscriber queue capacity.

Then add adversarial slow-reader tests that prove:

- response assembly, fragmented-frame reassembly, and writer queues each have an aggregate byte
  owner rather than only item-count bounds;
- cancellation, malformed transfers, disconnect, and shutdown release every reservation;
- the reserved priority headroom still lets collision-critical and control traffic preempt bulk
  terrain without an unbounded side queue or head-of-line deadlock.

## Make shutdown independent of client backpressure

`run_session` polls the process-shutdown watch at the top of its loop, but several selected branches
then await a bounded outbound-channel send. `write_frames` can simultaneously wait indefinitely on
the network sink. Once a slow client fills the channel, the session stops polling shutdown, Axum's
graceful shutdown waits for that session, and the SQLite checkpoint after `serve` may never run. The
existing writer-abort timeout begins only after the session loop exits, so it does not bound this
path.

Make both queue admission and socket writes process-shutdown-aware, without weakening delivery
ordering during normal operation. Add a deterministic stalled-sink test that fills the outbound
channel, triggers shutdown, and proves the session cancels its generation work, unregisters
presence/edit state, aborts the writer within its configured bound, reaches the checkpoint, and
returns from `serve_until` before a short deadline.

## Own automation setup processes before awaiting them

`runScenario` races the scenario promise against its abort signal, but raw `execFileAsync` children
are not part of that cancellation tree. `prepareWorldFixture` allocates a temporary world, awaits an
unowned Cargo process, and only then registers cleanup. Storage benchmarks have the same pattern for
build/native commands. If timeout or SIGINT arrives during setup, the child and descendants keep
running, the temporary world can leak, and the detached scenario may later attempt to register
cleanup after cleanup has already completed.

Route long-running setup commands through the existing process-tree-owned runner and register each
temporary path for cleanup immediately after allocation. Add a short-timeout fixture whose setup
command spawns a descendant, then prove the descendant is gone, the temporary world is removed, and
no late cleanup registration or unhandled rejection occurs.

## Recover active tabs after session-key rotation

The browser obtains one 12-hour session token during bootstrap and reloads five minutes before its
scheduled expiry. If `VOXELS_SESSION_SIGNING_KEY` rotates earlier, a tab that was already open keeps
retrying the invalid in-memory token until that reload or a manual refresh. A normal Fly deploy also
drops its sockets, so key rotation can leave otherwise healthy active tabs disconnected for most of
the token lifetime even though their durable identity credential remains valid.

Choose whether rotation uses an overlap window that verifies the previous session key or lets the
browser reauthorize and replace its token after an authentication-specific disconnect. Then prove:

- a tab opened before rotation reconnects both world and presence sockets within a bounded interval
  without changing its browser or player identity;
- ordinary transient failures do not cause authorization traffic or page-reload loops;
- staged Fly and Cloudflare updates never create an interval in which neither side accepts the same
  session key.

## Validate browser URLs without growing the client

The portable client-config validator accepts some authorities that the browser rejects, including
`ws://999.999.999.999/` and `ws://[not-an-ip]/`. The world-service origin validator similarly accepts
malformed authorities and user information. A prototype using the standards-compatible `url` crate
fixed those cases, but pulled Unicode host processing into the shipped WASM: the optimized module
grew from 6,502,233 to 6,726,961 bytes (+224,728, 3.5%), and local gzip output grew from 1,701,951 to
1,779,656 bytes (+77,705, 4.6%). That regression is too large for startup-only validation.

Choose a lightweight parser or move canonical browser parsing to a boundary that preserves Rust's
host-testable configuration contract. Require a shared corpus covering DNS, IPv4, bracketed IPv6,
ports, user information, paths, queries, fragments, escapes, and whitespace; compare every client
case against the browser `URL`/`WebSocket` constructors; and measure optimized raw and compressed
WASM size before landing it.

## Decide exact-corner voxel ray semantics

The portable DDA traversal in `core/src/lib.rs` advances one axis when two or three boundary times
are exactly equal. That is deterministic, but it chooses a voxel reached only through a mathematical
edge or corner before the other tied neighbors.

Changing it requires a gameplay decision: picking and visibility can use the current thin-ray rule,
or a conservative supercover rule can visit every tied neighbor. Define the expected behavior for
placing, digging, and occlusion at exact grid corners, then add symmetric positive/negative tie
fixtures before changing traversal.

## Prove cross-directory terrain root coherence

The `world-lab` procedural-v16 fixture can fill the encoded cache while the exact terrain envelope
remains at zero coverage. A bounded publication-frontier prototype made the next blocker explicit:
the renderer rejected the level-3 children of surface root
`TerrainPageKey { level: 4, coord: [-1, i32::MIN, -1] }` as an incoherent replacement. At that
point the CPU cut reported one ownerless root and three skipped-level edges, so publishing the
candidate would violate the single-cut ownership contract.

Do not solve this by admitting the children independently or weakening replacement validation.
First add a world-service fixture that generates the parent and all four child-root pages through
their real directory requests, then compare their persisted schema-v11 boundary witnesses and
reconstructed shared edges. The eventual fix belongs in generation/persistence if those products
disagree; renderer admission should continue failing closed. Re-run `world-lab` through exact
coverage and GPU certification before revisiting bounded staging retention.

The default `player-rendering` dev-server scenario exposes the other side of the same staging
problem with Terrain Diffusion: startup fails while admitting
`TerrainPageKey { level: 2, coord: [-2, i32::MIN, -7] }` because the virtual-terrain GPU page pool
is exhausted. A bounded frontier must therefore prove both liveness and a hard memory bound for the
published cut, its causal balancing closure, and one maximum replacement group. The direct-child
prototype bounded memory but did not converge; retaining every historical descendant progressed
farther but exposed the incoherent root above and is not safe to ship.

## Cancel completed browser request timeouts

Every streamed world request schedules a JavaScript timeout for the full request deadline, but
`schedule_after` discards the timeout handle. Successful, canceled, and preempted requests leave
their closure and timer registered until that deadline expires, even though the pending request has
already been removed. A continuously streaming client can therefore retain approximately ten
seconds of completed-request callbacks and later wake them in no-op bursts.

The fix needs one owner for each timer handle across chunk, terrain-directory, terrain-column, and
terrain-page request lifecycles rather than a local timeout-helper change. Before landing it, add
injectable timer scheduling/cancellation tests that prove exactly-once cleanup after success,
explicit cancellation, priority preemption, timeout, scheduling failure, and socket close. Then run
a sustained high-throughput stream and compare retained callbacks or heap growth plus callback CPU
before and after the change.
