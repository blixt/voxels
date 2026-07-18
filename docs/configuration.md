# Configuration

Voxels has two versioned TOML configuration files with deliberately separate ownership:

- `config/client.toml` contains presentation, local streaming, diagnostics, and profiling settings
  used by a game client.
- `config/world-service.toml` contains world identity and provider settings owned by the world
  service. Clients never read it and never branch on the selected provider.

The schemas and their consuming subsystems reject unknown fields, malformed values, unsupported
schema versions, and values outside bounded runtime ranges. Configuration is loaded and validated
once during startup, then passed into subsystems as immutable typed Rust values. Runtime code does
not read files or process environment variables on demand.

## Client configuration

The browser fetches `config/client.toml` before creating the engine worker and passes the unchanged
text across the startup boundary. `voxels-client-config` deserializes and validates it in Rust. A
bad or missing file prevents startup with a visible error instead of silently selecting defaults.

The Vite development server reloads the page when the file changes. Production builds copy it to
`dist/config/client.toml` as a separate deployment file rather than embedding it in JavaScript, so
an operator can tune a deployed client without rebuilding WASM. Reload the page after changing it.

The file controls:

- the authoritative world and presence endpoints, authorization token, and backpressure windows;
- pose cadence, clock synchronization, adaptive interpolation bounds, and extrapolation horizon;
- fixed-step timing, catch-up limit, and edit-tracker capacity;
- chunk and surface-LOD load/retention radii, pipeline budgets, interest capacity, and deterministic
  canonical-chunk view-cone/velocity look-ahead priority;
- view/shadow settings;
- the fixed rendering feature baseline and the World Lab's initial open state;
- whether local developer controls expose time/weather visualization overrides and the
  server-authorized creative-flight request;
- bounded diagnostic probe sizes and cadence;
- automated profile speed and warmup/measurement durations.

The World Lab deliberately does not expose ordinary renderer feature toggles. It keeps operational
rendering policy in the file and reserves the in-game surface for useful play/debug controls: local
time and weather previews plus creative flight when both client developer controls and the server's
gameplay capability allow it. Time/weather previews do not mutate the shared environment.

Player identity is intentionally not client configuration. The browser keeps a versioned local
registry: `/` selects its stable default player, while `/?player=alice` selects or creates a named
local player. All names under one browser profile share deployment configuration. Camera position,
inventory, and world edits belong to the native service and are shared across every interested
browser profile. See
[Native world streaming](native-world-streaming.md#local-players-and-two-browser-testing).

## World-service configuration

`config/world-service.toml` controls the seed, the procedural/Terrain Diffusion provider toggle,
the shared day/weather clock, native edit database and commit-queue bound, presence cadence/admission bounds, and Terrain
Diffusion deployment settings such as precision, model cache, model-space origin, and world placement. See
[World service configuration](world-service-config.md) for commands and the complete server schema.

The browser has no embedded world-generation mode. It always negotiates the same provider-neutral
chunk and surface-LOD protocol with this service, so editing `source` and restarting the daemon is
the only experience switch. Reconnect refuses a changed manifest rather than mixing worlds.

Provider selection is fail-closed. A Terrain Diffusion selection without the native Metal feature,
Apple Metal, or the pinned verified model is an error; it never falls back to another world.

## What is intentionally not configurable

Configuration is for deployment and operational policy, not for values that define compatibility
or memory safety. The following remain code-level invariants:

- persisted schema and binary-format versions, magic bytes, wire tags, and hashes;
- voxel/chunk dimensions, GPU buffer layouts, shader ABI values, and hard allocation ceilings;
- authored world content, procedural-v16 generation formulas, landmarks, and route geometry;
- pinned Terrain Diffusion repository revision, weight hashes, tensor topology, normalization, and
  sampler/scheduler semantics.

Changing those requires a hard version bump rather than a TOML edit. Development builds accept only
their exact schema and protocol versions; they select a fresh versioned local namespace instead of
migrating older data. Player physics also remains in portable `core` for now: it will become
server-authoritative game-rule data when the simulation protocol exists, rather than being introduced
as an untrusted client-local override.

## Testing configuration-dependent code

Tests do not mutate process-wide environment variables or rewrite production TOML strings.

- Parser and loader contract tests read TOML and cover versioning, unknown fields, path resolution,
  round trips, and validation failures.
- Behavior tests construct typed config fixtures directly, changing only the field relevant to the
  test.
- TypeScript loader tests inject the file-fetch function and URL instead of replacing global
  `fetch`.

This keeps tests deterministic and parallel-safe while exercising the same typed configuration that
production code receives.
