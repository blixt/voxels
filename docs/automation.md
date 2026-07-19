# Automation scenarios

Voxels uses one composable automation system for behavioral tests, performance benchmarks, bot
loads, screenshots, traces, and video. Scenarios describe intent; shared mechanisms own process,
browser, world-service, network, artifact, and cleanup details.

## Command

```sh
vp run automation -- list
vp run automation -- run lod-transition
vp run automation -- run bot-load --counts=64 --no-browser
vp run automation -- run bot-load --counts=16 --duration=10 --video
vp run automation -- run spectator-feed --url=http://127.0.0.1:5173 --duration=30
vp run automation -- describe render-profile
```

`list` and `describe` expose each scenario's kind and declared mechanisms. `run` creates an isolated
run directory, records a manifest, installs signal-safe cleanup, and invokes the scenario. Scenarios
create temporary services, databases, and browser profiles by default. A scenario that deliberately
attaches to a running world requires an explicit URL; spectator feeds cannot edit terrain but may
create or update their named player identity in the service's ordinary player store.

Static checks remain ordinary Vite+ commands:

```sh
vp check
vp test
vp build
vp run check:rust
vp run verify
```

## Layout

```text
automation/
  cli.ts                  one command-line entry point
  scenarios/              readable intent and scenario-specific assertions
  lib/
    arguments.ts          strict reusable scenario option parsing
    scenario.ts           definition, registry contract, runner, cleanup
    artifacts.ts          run directories, JSON, screenshots, video, manifests
    browser.ts            Chromium lifecycle, pages, errors, screenshots, video
    engine.ts             typed Rust automation API and snapshot decoding
    world.ts              isolated service/client configs and daemon lifecycle
    metrics.ts            summaries and process sampling
    render-metrics.ts     frame, CPU, and GPU snapshot collection
    network.ts            deterministic shaped loopback links and accounting
    protocol.ts           versioned VXWP constants used by automation
```

Only build integration and the Rust static gate remain under `scripts/`. Model setup, native
benchmarks, provider validation, browser captures, network experiments, multiplayer checks, and bot
loads are scenarios. There are no `.mjs` files.

## Scenario contract

Every scenario declares what it is and which mechanisms it uses:

```ts
import { ScenarioArguments } from "../lib/arguments.ts";
import { BrowserCapability } from "../lib/browser.ts";
import { defineScenario } from "../lib/scenario.ts";
import { startWorldPreview } from "../lib/world.ts";

export default defineScenario({
  id: "weather-motion",
  kind: "validation",
  summary: "Clouds remain world-anchored and precipitation falls.",
  uses: {
    world: true,
    browser: true,
    viewport: "browser",
    screenshots: true,
    metrics: true,
  },
  async run(context, arguments_) {
    new ScenarioArguments(arguments_).assertEmpty();
    const world = await startWorldPreview(context, {
      fixture: { weatherCycleSeconds: 120 },
    });
    const browser = await BrowserCapability.start(context);
    const viewport = await browser.open({ url: world.url, label: "weather" });
    await viewport.screenshot("before");
    // Scenario-specific movement and assertions stay here.
    await viewport.screenshot("after");
    browser.assertHealthy();
    return { summary: "Weather motion passed." };
  },
});
```

`kind` is one of `validation`, `benchmark`, `capture`, `bot-load`, `analysis`, or `setup`. `uses` is
data, not documentation: the runner validates unavailable combinations and records the exact
mechanisms in the artifact manifest. Mechanisms are lazy, so a native bot or Criterion scenario does
not build WASM, launch Chrome, or allocate a viewport.

Scenario code receives a `ScenarioContext` with owned capabilities. Each capability registers cleanup
when it acquires a resource. Cleanup runs in reverse order on success, assertion failure, signal, or
child-process failure. Scenario files must not install their own process signal handlers.

## Artifacts and composition

Every run has one `target/automation/<scenario>/<run-id>/` directory containing `manifest.json` and
scenario outputs. `target/automation/<scenario>/latest.json` points to the last completed run while
timestamped results remain comparison-safe. Screenshots, raw video, browser traces, logs, and metric
reports all use the same artifact API.

A browser viewport, recorder, shaped link, service, and bot army are independent mechanisms. A
scenario may therefore launch bots behind a shaped link while a browser observes and records the
same world. `bot-load --video` is the concrete combined case. Video and screenshots capture a
viewport; they do not own one.

## Spectator feeds

`spectator-feed` turns the automation renderer into a reusable regional camera rather than a test
double. Without arguments it owns an isolated world. Pass an explicit URL to attach it to a running
development or deployed client:

```sh
vp dev
vp run automation -- run spectator-feed \
  --url=http://127.0.0.1:5173 \
  --player=coast-camera \
  --duration=30 \
  --motion=orbit \
  --look=1.2,-0.25
```

The scenario negotiates the server-authorized spectator role through Rust, confirms that both edit
entry points are inert, then captures 1920x1080 start/end frames and raw WebM video. `--motion` is
`stationary`, `forward`, `orbit`, or `rise`; `--no-video` produces screenshots only. An optional
`--session-state=path/to/state.json` preserves the isolated browser identity between runs. The feed
still has no server shortcut: terrain streams by camera interest and movement stays inside ordinary
pose budgets.

This is also the product boundary for later live outputs. A WebRTC, WebTransport, or broadcast
encoder can consume the spectator viewport without changing player simulation, world authority, or
the scenario API. Protocol-faithful helpful bots can reuse the existing native bot capability, but
production bots should receive explicit server-owned identities and narrowly scoped actions rather
than treating an automation browser as trusted authority.

## Rust engine boundary

Automation controls engine semantics through the Rust/WASM worker API, never by mutating browser
state or renderer internals. The boundary is versioned and runtime-checked:

- Rust owns the automation contract version and snapshot schema.
- Worker messages are a TypeScript discriminated union.
- The browser exposes one `EngineAutomationApi` interface.
- TypeScript decodes the compact numeric snapshot into named, readonly state only after validating
  the Rust schema version and exact field layout.
- Semantic actions such as look, spectator role changes, profile start, dig, place, inventory, and
  surface diagnostics use typed methods. New controls start in Rust and extend the worker union
  before scenarios can use them.

The numeric wire format remains allocation-conscious for high-frequency profiling, while scripts do
not contain raw indices.

## Vite+ test integration

`scenarioTest` runs the same scenario definition inside Vite+'s test framework with an isolated
artifact directory and test timeout. A single `.test.ts` file can define the world, capture a
screenshot, inspect typed engine state or pixels, and fail normally:

```ts
const scenario = defineScenario({
  /* setup, capture, assertions */
});
scenarioTest(scenario);
```

The CLI and test adapter both call the same runner. Tests do not shell out to the CLI.

## Viewport policy

Browser scenarios use the production worker, Rust engine, WGPU renderer, and WebGPU backend. Native
Rust benchmarks and bot loads already avoid browsers when no viewport is needed.

A future native viewport must instantiate the same renderer and scenario controls, then pass a
parity scenario that compares camera, selected-world fingerprint, draw counts, framebuffer size, and
pixel evidence against the browser backend. Until that exact path exists, automation reports native
viewport as unavailable instead of maintaining a cheaper renderer clone whose behavior can drift.

## Adding a scenario

1. Add one TypeScript file under `automation/scenarios/`.
2. Declare `id`, `kind`, `summary`, and `uses`.
3. Use context capabilities instead of spawning shared infrastructure directly.
4. Keep only scenario-specific actions, metrics, and assertions in the file.
5. Run `vp run automation -- describe <id>`, then the scenario and `vp check`. Discovery is
   automatic; adding a second registry entry is intentionally unnecessary.
