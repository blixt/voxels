# Automation scenarios

Voxels uses one composable automation system for behavioral tests, performance benchmarks, bot
loads, screenshots, traces, and video. Scenarios describe intent; shared mechanisms own process,
browser, world-service, network, artifact, and cleanup details.

## Command

```sh
vp run automation -- list
vp run automation -- run lod-transition
vp run automation -- run bot-load --counts=64 --no-browser
vp run automation -- describe render-profile
```

`list` and `describe` expose each scenario's kind and declared mechanisms. `run` creates an isolated
run directory, records a manifest, installs signal-safe cleanup, and invokes the scenario. It never
uses the development database or browser profile.

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
    scenario.ts           definition, registry contract, runner, cleanup
    artifacts.ts          run directories, JSON, screenshots, video, manifests
    browser.ts            Chromium lifecycle, pages, errors, screenshots, traces
    engine.ts             typed Rust automation API and snapshot decoding
    world.ts              isolated service/client configs and daemon lifecycle
    bots.ts               native bot-army process capability
    metrics.ts            summaries and process sampling
    network.ts            deterministic shaped loopback links and accounting
    protocol.ts           versioned VXWP constants used by automation
```

Build, Rust-check, Terrain Diffusion model-management, and daemon convenience scripts are repository
tooling rather than scenarios and remain under `scripts/`. There are no `.mjs` files.

## Scenario contract

Every scenario declares what it is and which mechanisms it uses:

```ts
export default defineScenario({
  id: "weather-motion",
  kind: "validation",
  summary: "Clouds remain world-anchored and precipitation falls.",
  uses: {
    world: true,
    viewport: "browser",
    screenshots: true,
    metrics: true,
  },
  async run(context, arguments_) {
    const world = await context.world.start({ weather: "accelerated" });
    const page = await context.browser.open({ world });
    const before = await page.screenshot("before");
    // Scenario-specific movement and assertions stay here.
    await page.screenshot("after");
    return context.pass({ before });
  },
});
```

`kind` is one of `validation`, `benchmark`, `capture`, `bot-load`, or `analysis`. `uses` is data, not
documentation: the runner validates unavailable combinations and records the exact mechanisms in the
artifact manifest. Mechanisms are lazy, so a native bot or Criterion scenario does not build WASM,
launch Chrome, or allocate a viewport.

Scenario code receives a `ScenarioContext` with owned capabilities. Each capability registers cleanup
when it acquires a resource. Cleanup runs in reverse order on success, assertion failure, signal, or
child-process failure. Scenario files must not install their own process signal handlers.

## Artifacts and composition

Every run has one `target/automation/<scenario>/<run-id>/` directory containing `manifest.json` and
scenario outputs. Stable `latest.json` pointers are allowed for local iteration, while timestamped
results remain comparison-safe. Screenshots, raw video, annotated video, browser traces, logs, and
metric reports all use the same artifact API.

A browser viewport, recorder, shaped link, service, and bot army are independent capabilities. A
scenario may therefore launch bots behind a shaped link while a browser observes and records the
same world. Video and screenshots capture a viewport; they do not own one.

## Rust engine boundary

Automation controls engine semantics through the Rust/WASM worker API, never by mutating browser
state or renderer internals. The boundary is versioned and runtime-checked:

- Rust owns the automation contract version and snapshot schema.
- Worker messages are a TypeScript discriminated union.
- The browser exposes one `EngineAutomationApi` interface.
- TypeScript decodes the compact numeric snapshot into named, readonly state only after validating
  the Rust schema version and exact field layout.
- Semantic actions such as look, profile start, dig, place, inventory, and surface diagnostics use
  typed methods. New controls start in Rust and extend the worker union before scenarios can use them.

The numeric wire format remains allocation-conscious for high-frequency profiling, while scripts do
not contain raw indices.

## Vite+ test integration

`scenarioTest` runs the same scenario definition inside Vite+'s test framework with an isolated
artifact directory and test timeout. A single `.test.ts` file can define the world, capture a
screenshot, inspect typed engine state or pixels, and fail normally:

```ts
const scenario = defineScenario({ /* setup, capture, assertions */ });
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
5. Register it in the typed scenario registry.
6. Run `vp run automation -- describe <id>`, then the scenario and `vp check`.

