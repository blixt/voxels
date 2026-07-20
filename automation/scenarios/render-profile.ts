import { execFileSync } from "node:child_process";
import { mkdir, open } from "node:fs/promises";
import path from "node:path";
import type { CDPSession, Page } from "playwright";
import { ScenarioArguments } from "../lib/arguments.ts";
import { BrowserCapability, chromeWebGpuLaunchOptions } from "../lib/browser.ts";
import { type EngineClient, snapshotValue } from "../lib/engine.ts";
import {
  captureRenderSnapshot,
  sampleRenderSnapshots,
  summarizeRenderPhase,
  type RenderPhaseSummary,
  type RenderSnapshotCapture,
} from "../lib/render-metrics.ts";
import { defineScenario, type ScenarioContext } from "../lib/scenario.ts";
import { startWorldStack } from "../lib/world.ts";
import type { WorldSource } from "../lib/world.ts";
import type { WasmBuildProfile } from "../../scripts/build-wasm.ts";

const FAILURE =
  /panic|unreachable|runtimeerror|wgpu|webgpu|shader|sqlite|opfs|syncaccesshandle|nomodificationallowed|web lock request failed|no persistence leader|persistence .*failed/i;

function missed120HzFrameGate(phase: RenderPhaseSummary): boolean {
  // Use sustained percentiles for the display-paced gate while still rejecting
  // any isolated hitch large enough to skip more than one 120 Hz presentation.
  return phase.frameMs.p95 > 12 || phase.frameMs.p99 > 16.67 || phase.frameMs.max > 20;
}

async function mark(page: Page, name: string): Promise<void> {
  await page.evaluate((value) => performance.mark(value), name);
}

async function startChromiumTrace(
  context: import("playwright").BrowserContext,
  page: Page,
): Promise<CDPSession> {
  const session = await context.newCDPSession(page);
  await session.send("Performance.enable");
  await session.send("Tracing.start", {
    transferMode: "ReturnAsStream",
    traceConfig: {
      recordMode: "recordContinuously",
      enableSampling: true,
      includedCategories: [
        "blink.user_timing",
        "cc",
        "devtools.timeline",
        "disabled-by-default-devtools.timeline",
        "disabled-by-default-v8.cpu_profiler",
        "gpu",
        "renderer.scheduler",
        "toplevel",
        "v8",
        "viz",
      ],
    },
  });
  return session;
}

async function stopChromiumTrace(
  session: CDPSession,
  outputPath: string,
): Promise<Record<string, number>> {
  const completed = new Promise<{ stream?: string }>((resolve) =>
    session.once("Tracing.tracingComplete", resolve),
  );
  await session.send("Tracing.end");
  const { stream } = await completed;
  if (!stream) throw new Error("Chromium trace completed without a readable stream");
  await mkdir(path.dirname(outputPath), { recursive: true });
  const file = await open(outputPath, "w");
  try {
    while (true) {
      const chunk = await session.send("IO.read", { handle: stream });
      const bytes = Buffer.from(chunk.data, chunk.base64Encoded ? "base64" : "utf8");
      await file.write(bytes);
      if (chunk.eof) break;
    }
  } finally {
    await file.close();
    await session.send("IO.close", { handle: stream });
  }
  const metrics = await session.send("Performance.getMetrics");
  await session.detach();
  return Object.fromEntries(metrics.metrics.map(({ name, value }) => [name, value]));
}

function requiredGpu(distribution: RenderPhaseSummary["gpu"]["totalMs"], label: string) {
  if (distribution === null) throw new Error(`${label} GPU timestamps are unavailable`);
  return distribution;
}

async function setMaterialDetail(
  page: Page,
  engine: EngineClient,
  enabled: boolean,
  viewportWidth: number,
): Promise<void> {
  const current = (await engine.value("materialDetail")) === 1;
  if (current === enabled) return;
  await page.keyboard.press("F3");
  await page.waitForTimeout(200);
  // Material detail is the eighth Rust-owned feature row. The click targets the toggle,
  // exercising the same canvas hit-testing path as a human rather than a JavaScript render option.
  await page.mouse.click(viewportWidth - 57, 607);
  await engine.waitForSnapshot(
    (snapshot) => (snapshotValue(snapshot, "materialDetail") === 1) === enabled,
    { description: `material detail did not become ${enabled ? "enabled" : "disabled"}` },
  );
  await page.keyboard.press("F3");
  await page.waitForTimeout(800);
}

async function materialDetailProfile(page: Page, engine: EngineClient, viewportWidth: number) {
  await setMaterialDetail(page, engine, false, viewportWidth);
  const off = summarizeRenderPhase(await sampleRenderSnapshots(engine, 5_000));
  await setMaterialDetail(page, engine, true, viewportWidth);
  const on = summarizeRenderPhase(await sampleRenderSnapshots(engine, 5_000));
  const offWorld = requiredGpu(off.gpu.worldMs, "material detail off world");
  const onWorld = requiredGpu(on.gpu.worldMs, "material detail on world");
  const offTotal = requiredGpu(off.gpu.totalMs, "material detail off total");
  const onTotal = requiredGpu(on.gpu.totalMs, "material detail on total");
  const delta = {
    worldP95Ms: onWorld.p95 - offWorld.p95,
    totalP95Ms: onTotal.p95 - offTotal.p95,
    frameP95Ms: on.frameMs.p95 - off.frameMs.p95,
    coreGpuMiB: on.coreGpuMiB - off.coreGpuMiB,
  };
  const invariantKeys = [
    "residentChunks",
    "quads",
    "waterQuads",
    "waterDrawCalls",
    "drawCalls",
    "meshArenaAllocatedMiB",
    "meshArenaCapacityMiB",
    "refractionCopyMiB",
  ] as const satisfies readonly (keyof RenderPhaseSummary)[];
  const changed = invariantKeys.filter((key) => on[key] !== off[key]);
  const violations: string[] = [];
  if (!off.gpu.available || !on.gpu.available) violations.push("GPU timestamps unavailable");
  if (off.materialDetail || !on.materialDetail)
    violations.push("Rust UI toggle state was not observed");
  if (changed.length > 0)
    violations.push(`geometry/resource invariants changed: ${changed.join(", ")}`);
  if (delta.worldP95Ms > 0.5) violations.push("world GPU p95 increased by more than 0.50ms");
  if (delta.totalP95Ms > 0.75) violations.push("active GPU p95 increased by more than 0.75ms");
  if (off.frameMs.p95 > 12 || on.frameMs.p95 > 12) violations.push("frame p95 exceeded 12ms");
  if (off.frameMs.above33_33ms > 0 || on.frameMs.above33_33ms > 0) {
    violations.push("a paired profile frame exceeded 33.33ms");
  }
  if (off.droppedSamples > 0 || on.droppedSamples > 0)
    violations.push("frame samples were dropped");
  const result = { off, on, delta, invariantKeys };
  if (violations.length > 0) {
    throw new Error(
      `material detail profile violations: ${violations.join(", ")}; ${JSON.stringify(result)}`,
    );
  }
  return result;
}

async function waitForDayFraction(engine: EngineClient, target: number): Promise<void> {
  await engine.waitForSnapshot(
    (snapshot) => {
      const distance = Math.abs(snapshotValue(snapshot, "dayFraction") - target);
      return Math.min(distance, 1 - distance) <= 0.012;
    },
    { timeoutMs: 60_000, intervalMs: 50, description: `day fraction did not reach ${target}` },
  );
}

async function atmosphereProfile(page: Page, engine: EngineClient, context: ScenarioContext) {
  const anchors = [
    ["midnight", 0.0],
    ["dawn", 0.235],
    ["noon", 0.5],
    ["goldenHour", 0.72],
    ["blueHour", 0.8],
  ] as const;
  type AnchorName = (typeof anchors)[number][0];
  const phases: Partial<Record<AnchorName, RenderPhaseSummary>> = {};
  for (const [name, dayFraction] of anchors) {
    await waitForDayFraction(engine, dayFraction);
    const screenshot = context.artifacts.resolve(`atmosphere-${name}.png`);
    await page.screenshot({ path: screenshot });
    context.artifacts.record(`atmosphere ${name}`, screenshot, "image/png");
    phases[name] = summarizeRenderPhase(await sampleRenderSnapshots(engine, 800));
  }

  const phase = (name: AnchorName): RenderPhaseSummary => {
    const value = phases[name];
    if (value === undefined) throw new Error(`atmosphere omitted ${name}`);
    return value;
  };
  const values = Object.values(phases);
  const violations: string[] = [];
  if (values.length !== anchors.length) violations.push("did not observe every day-cycle anchor");
  const reference = values[0];
  for (const [name, phase] of Object.entries(phases)) {
    if (missed120HzFrameGate(phase)) {
      violations.push(`${name} missed the 120Hz frame gate`);
    }
    if (phase.gpu.available && requiredGpu(phase.gpu.worldMs, `${name} world`).p95 > 4) {
      violations.push(`${name} world GPU p95 exceeded 4ms`);
    }
    if (phase.gpu.available && requiredGpu(phase.gpu.totalMs, `${name} total`).p95 > 7.5) {
      violations.push(`${name} active GPU p95 exceeded 7.5ms`);
    }
    if (phase.atmosphere.cloudCoverage < 0.08 || phase.atmosphere.cloudCoverage > 0.94) {
      violations.push(`${name} cloud coverage escaped its normalized visual range`);
    }
    if (
      !Number.isFinite(phase.atmosphere.localSolarDayFraction) ||
      phase.atmosphere.localSolarDayFraction < 0 ||
      phase.atmosphere.localSolarDayFraction >= 1 ||
      !Number.isFinite(phase.atmosphere.yearFraction) ||
      phase.atmosphere.yearFraction < 0 ||
      phase.atmosphere.yearFraction >= 1 ||
      !Number.isFinite(phase.atmosphere.moonOrbitFraction) ||
      phase.atmosphere.moonOrbitFraction < 0 ||
      phase.atmosphere.moonOrbitFraction >= 1 ||
      !Number.isFinite(phase.atmosphere.moonIlluminatedFraction) ||
      phase.atmosphere.moonIlluminatedFraction < 0 ||
      phase.atmosphere.moonIlluminatedFraction > 1
    ) {
      violations.push(`${name} celestial telemetry escaped its normalized range`);
    }
    const bodyDirections: readonly (readonly [string, readonly [number, number, number]])[] = [
      ["sun", phase.atmosphere.sunDirection],
      ["moon", phase.atmosphere.moonDirection],
    ];
    for (const [body, direction] of bodyDirections) {
      const length = Math.hypot(...direction);
      if (!Number.isFinite(length) || Math.abs(length - 1) > 0.001) {
        violations.push(`${name} ${body} direction was not normalized`);
      }
    }
    if (
      reference &&
      (phase.quads !== reference.quads ||
        phase.visibleChunks !== reference.visibleChunks ||
        phase.meshArenaCapacityMiB !== reference.meshArenaCapacityMiB)
    ) {
      violations.push(`${name} changed geometry or mesh residency`);
    }
    if (phase.droppedSamples > 0) violations.push(`${name} dropped frame samples`);
  }
  if (phase("midnight").atmosphere.sunDirection[1] >= 0) {
    violations.push("midnight sun did not move below the horizon");
  }
  if (phase("noon").atmosphere.sunDirection[1] <= 0.9) {
    violations.push("noon sun did not reach its high arc");
  }
  if (phase("midnight").shadowCascades !== 0 || phase("midnight").shadowDrawCalls !== 0) {
    violations.push("midnight still rendered directional shadow cascades");
  }
  if (phase("noon").shadowCascades === 0 || phase("noon").shadowDrawCalls === 0) {
    violations.push("noon did not render directional shadow cascades");
  }
  if (violations.length > 0) {
    throw new Error(
      `atmosphere profile violations: ${violations.join(", ")}; ${JSON.stringify(phases)}`,
    );
  }
  return phases;
}

async function waitForWeatherFraction(engine: EngineClient, target: number): Promise<void> {
  await engine.waitForSnapshot(
    (snapshot) => {
      const distance = Math.abs(snapshotValue(snapshot, "weatherFraction") - target);
      return Math.min(distance, 1 - distance) <= 0.012;
    },
    { timeoutMs: 60_000, intervalMs: 50, description: `weather fraction did not reach ${target}` },
  );
}

async function weatherProfile(page: Page, engine: EngineClient, context: ScenarioContext) {
  const anchors = [
    ["clear", 0.08],
    ["cloudy", 0.23],
    ["overcast", 0.32],
    ["rain", 0.5],
    ["storm", 0.68],
    ["clearing", 0.89],
  ] as const;
  type AnchorName = (typeof anchors)[number][0];
  const phases: Partial<Record<AnchorName, RenderPhaseSummary>> = {};
  for (const [name, weatherFraction] of anchors) {
    await waitForWeatherFraction(engine, weatherFraction);
    const screenshot = context.artifacts.resolve(`weather-${name}.png`);
    await page.screenshot({ path: screenshot });
    context.artifacts.record(`weather ${name}`, screenshot, "image/png");
    phases[name] = summarizeRenderPhase(await sampleRenderSnapshots(engine, 800));
  }

  const phase = (name: AnchorName): RenderPhaseSummary => {
    const value = phases[name];
    if (value === undefined) throw new Error(`weather omitted ${name}`);
    return value;
  };
  const violations: string[] = [];
  const reference = phase("clear");
  for (const [name, phase] of Object.entries(phases)) {
    if (missed120HzFrameGate(phase)) {
      violations.push(`${name} missed the 120Hz frame gate`);
    }
    const cloudGpu = phase.gpu.available ? requiredGpu(phase.gpu.cloudMs, `${name} cloud`) : null;
    if (cloudGpu !== null && (cloudGpu.median > 1.7 || cloudGpu.p95 > 3)) {
      violations.push(`${name} cloud GPU exceeded the 1.7ms median / 3ms p95 budget`);
    }
    if (phase.gpu.available && requiredGpu(phase.gpu.totalMs, `${name} total`).p95 > 7.5) {
      violations.push(`${name} active GPU p95 exceeded 7.5ms`);
    }
    if (
      phase.quads !== reference.quads ||
      phase.visibleChunks !== reference.visibleChunks ||
      phase.meshArenaCapacityMiB !== reference.meshArenaCapacityMiB
    ) {
      violations.push(`${name} changed geometry or mesh residency`);
    }
    if (phase.atmosphere.cloudRenderResolution.join("x") !== "640x360") {
      violations.push(`${name} did not render clouds at the configured half resolution`);
    }
    if (phase.droppedSamples > 0) violations.push(`${name} dropped frame samples`);
  }
  if (phase("clear").atmosphere.precipitation > 0.01) {
    violations.push("clear weather still produced precipitation");
  }
  if (phase("rain").atmosphere.precipitation < 0.45) {
    violations.push("rain did not produce substantial precipitation");
  }
  if (phase("storm").atmosphere.storminess < 0.7) {
    violations.push("storm anchor did not produce a storm");
  }
  if (phase("storm").atmosphere.fogDensity <= phase("clear").atmosphere.fogDensity) {
    violations.push("storm did not thicken atmospheric fog");
  }
  if (phase("storm").atmosphere.shadowStrength >= phase("clear").atmosphere.shadowStrength) {
    violations.push("storm did not soften directional shadows");
  }
  if (violations.length > 0) {
    throw new Error(
      `weather profile violations: ${violations.join(", ")}; ${JSON.stringify(phases)}`,
    );
  }
  return phases;
}

async function sustainedProfile(engine: EngineClient, profileId = 1) {
  await engine.startProfile(profileId);
  const captures: RenderSnapshotCapture[] = [];
  const deadline = Date.now() + 120_000;
  while (Date.now() < deadline) {
    const next = await captureRenderSnapshot(engine);
    captures.push(next);
    if (snapshotValue(next.snapshot, "profileComplete") === 1) break;
    await engine.wait(250);
  }
  const latest = captures.at(-1)?.snapshot;
  if (latest === undefined || snapshotValue(latest, "profileComplete") !== 1) {
    throw new Error(
      `sustained Rust profile did not drain: ${JSON.stringify(latest?.slice(56, 70))}`,
    );
  }
  const measured = captures.filter(
    (capture) => snapshotValue(capture.snapshot, "profilePhase") === 2,
  );
  const finalTwenty = measured.filter(
    (capture) => snapshotValue(capture.snapshot, "profileElapsedSeconds") >= 70,
  );
  const moving = captures.filter((capture) => {
    const phase = snapshotValue(capture.snapshot, "profilePhase");
    return phase === 1 || phase === 2;
  });
  const atSixtySeconds = moving.reduce<RenderSnapshotCapture | undefined>((closest, capture) => {
    if (closest === undefined) return capture;
    const error = Math.abs(snapshotValue(capture.snapshot, "profileElapsedSeconds") - 60);
    const closestError = Math.abs(snapshotValue(closest.snapshot, "profileElapsedSeconds") - 60);
    return error < closestError ? capture : closest;
  }, undefined);
  const range = (values: readonly number[]): number =>
    values.length === 0 ? 0 : Math.max(...values) - Math.min(...values);
  const result = {
    measured: summarizeRenderPhase(measured),
    distanceMetres: snapshotValue(latest, "profileDistanceMetres"),
    evictions: snapshotValue(latest, "profileEvictions"),
    highWater: {
      trackedChunks: snapshotValue(latest, "profileTrackedHigh"),
      surfaceTiles: snapshotValue(latest, "profileSurfaceHigh"),
      pendingJobs: snapshotValue(latest, "profilePendingHigh"),
      pendingMeshes: snapshotValue(latest, "profilePendingMeshHigh"),
      arenaCapacityMiB: snapshotValue(latest, "profileArenaCapacityHighMiB"),
      wasmCommittedMiB: snapshotValue(latest, "profileWasmHighMiB"),
    },
    finalTwentySeconds: {
      wasmCommittedRangeMiB: range(
        finalTwenty.map((capture) => snapshotValue(capture.snapshot, "wasmCommittedMiB")),
      ),
      arenaCapacityRangeMiB: range(
        finalTwenty.map((capture) => snapshotValue(capture.snapshot, "arenaCapacityMiB")),
      ),
    },
    lod: {
      samples: moving.length,
      degradedSamples: moving.filter(
        (capture) => snapshotValue(capture.snapshot, "presentedLodStrideVoxels") > 1,
      ).length,
      missingSamples: moving.filter(
        (capture) => snapshotValue(capture.snapshot, "presentedLodStrideVoxels") === 0,
      ).length,
      maximumStrideVoxels: Math.max(
        0,
        ...moving.map((capture) => snapshotValue(capture.snapshot, "presentedLodStrideVoxels")),
      ),
      maximumFocusLagVoxels: Math.max(
        0,
        ...moving.map((capture) => snapshotValue(capture.snapshot, "lodFocusLagVoxels")),
      ),
      canonicalImmediateReadySamples: moving.filter(
        (capture) =>
          snapshotValue(capture.snapshot, "canonicalImmediateRequired") > 0 &&
          snapshotValue(capture.snapshot, "canonicalImmediateResident") ===
            snapshotValue(capture.snapshot, "canonicalImmediateRequired"),
      ).length,
      atSixtySeconds:
        atSixtySeconds === undefined
          ? null
          : {
              elapsedSeconds: snapshotValue(atSixtySeconds.snapshot, "profileElapsedSeconds"),
              distanceMetres: snapshotValue(atSixtySeconds.snapshot, "profileDistanceMetres"),
              presentedStrideVoxels: snapshotValue(
                atSixtySeconds.snapshot,
                "presentedLodStrideVoxels",
              ),
              focusLagVoxels: snapshotValue(atSixtySeconds.snapshot, "lodFocusLagVoxels"),
              canonicalImmediateResident: snapshotValue(
                atSixtySeconds.snapshot,
                "canonicalImmediateResident",
              ),
              canonicalImmediateRequired: snapshotValue(
                atSixtySeconds.snapshot,
                "canonicalImmediateRequired",
              ),
              pendingJobs: snapshotValue(atSixtySeconds.snapshot, "pendingJobs"),
            },
    },
    final: {
      pendingJobs: snapshotValue(latest, "pendingJobs"),
      pendingMeshMiB: snapshotValue(latest, "pendingMeshMiB"),
      canonicalVoxelMiB: snapshotValue(latest, "canonicalVoxelMiB"),
      staleCompletions: snapshotValue(latest, "staleCompletions"),
    },
  };
  const violations: string[] = [];
  if (result.distanceMetres < 1_000) violations.push("distance below 1km");
  if (result.evictions < 500) violations.push("fewer than 500 canonical evictions");
  if (result.highWater.trackedChunks > 320) violations.push("tracked chunk bound exceeded");
  if (result.highWater.surfaceTiles > 896) violations.push("surface residency bound exceeded");
  if (result.highWater.pendingMeshes > 3) violations.push("pending mesh bound exceeded");
  if (result.final.pendingJobs !== 0) violations.push("queues did not drain");
  if (result.final.pendingMeshMiB !== 0) violations.push("pending mesh payload did not drain");
  if (result.measured.frameMs.p95 > 12) violations.push("frame p95 above 12ms");
  if (result.measured.frameMs.p99 > 16.67) violations.push("frame p99 above 16.67ms");
  if (result.measured.cpuMs.p95 > 7.5) violations.push("worker CPU p95 above 7.5ms");
  if (result.measured.streamingMs.p95 > 4.5) violations.push("streaming p95 above 4.5ms");
  if (result.measured.frameMs.above33_33ms > 0) violations.push("frame exceeded 33.33ms");
  if (result.measured.droppedSamples > 0) violations.push("frame samples were dropped");
  if (result.finalTwentySeconds.wasmCommittedRangeMiB > 1) {
    violations.push("WASM committed memory did not plateau");
  }
  if (result.finalTwentySeconds.arenaCapacityRangeMiB > 4) {
    violations.push("mesh arena capacity did not plateau");
  }
  if (violations.length > 0) {
    throw new Error(
      `sustained profile violations: ${violations.join(", ")}; ${JSON.stringify(result)}`,
    );
  }
  return result;
}

async function waitForEngine(engine: EngineClient): Promise<readonly number[]> {
  return engine.waitForSnapshot(
    (snapshot) =>
      snapshotValue(snapshot, "quads") > 0 &&
      snapshotValue(snapshot, "residentChunks") > 0 &&
      snapshotValue(snapshot, "pendingJobs") === 0,
    { timeoutMs: 60_000, intervalMs: 50, description: "release engine did not settle" },
  );
}

type ProfileMode =
  | "standard"
  | "stationary"
  | "sustained"
  | "directional"
  | "materials"
  | "atmosphere"
  | "weather";

interface ProfileOptions {
  readonly mode: ProfileMode;
  readonly trace: boolean;
  readonly screenshot: boolean;
  readonly worldSource: WorldSource;
  readonly spawnVoxels?: readonly [number, number];
  readonly cameraLook?: readonly [number, number];
  readonly viewport: { readonly width: number; readonly height: number };
  readonly deviceScaleFactor: number;
  readonly cascadedShadows: boolean;
  readonly screenSpaceAmbientOcclusion: boolean;
  readonly fixedDayFraction?: number;
  readonly fixedWeatherFraction?: number;
  readonly buildProfile: WasmBuildProfile;
}

function parseOptions(arguments_: readonly string[]): ProfileOptions {
  const argumentsReader = new ScenarioArguments(arguments_);
  const mode = argumentsReader.choice(
    "mode",
    [
      "standard",
      "stationary",
      "sustained",
      "directional",
      "materials",
      "atmosphere",
      "weather",
    ] as const,
    "standard",
  );
  const viewportValues = argumentsReader.pair("viewport", {
    fallback: [1280, 720],
    separator: "x",
    integer: true,
    minimum: 240,
  });
  if (viewportValues === undefined) throw new Error("viewport default is missing");
  const spawnVoxels = argumentsReader.pair("spawn", {
    integer: true,
    minimum: -2_147_483_648,
    maximum: 2_147_483_647,
  });
  const cameraLook = argumentsReader.pair("look", {
    minimum: -Math.PI * 2,
    maximum: Math.PI * 2,
  });
  if (cameraLook !== undefined && (cameraLook[1] < -Math.PI / 2 || cameraLook[1] > Math.PI / 2)) {
    throw new Error("--look pitch must be in -pi/2..=pi/2");
  }
  const shadows = argumentsReader.choice("shadows", ["on", "off"] as const, "on");
  const ambientOcclusion = argumentsReader.choice("ssao", ["on", "off"] as const, "on");
  const fixedDayFraction = argumentsReader.number("day-fraction", {
    minimum: 0,
    maximum: 0.999_999,
  });
  const fixedWeatherFraction = argumentsReader.number("weather-fraction", {
    minimum: 0,
    maximum: 0.999_999,
  });
  const options: ProfileOptions = {
    mode,
    trace: argumentsReader.flag("trace"),
    screenshot: argumentsReader.flag("screenshot"),
    worldSource: argumentsReader.choice(
      "source",
      ["procedural-v16", "terrain-diffusion-30m"] as const,
      "procedural-v16",
    ),
    ...(spawnVoxels === undefined ? {} : { spawnVoxels }),
    ...(cameraLook === undefined ? {} : { cameraLook }),
    viewport: { width: viewportValues[0], height: viewportValues[1] },
    deviceScaleFactor:
      argumentsReader.number("dpr", { fallback: 1, minimum: 0.5, maximum: 4 }) ?? 1,
    cascadedShadows: shadows === "on",
    screenSpaceAmbientOcclusion: ambientOcclusion === "on",
    ...(fixedDayFraction === undefined ? {} : { fixedDayFraction }),
    ...(fixedWeatherFraction === undefined ? {} : { fixedWeatherFraction }),
    buildProfile: argumentsReader.choice(
      "build",
      ["debug", "wasm-dev", "release"] as const,
      "release",
    ),
  };
  argumentsReader.assertEmpty();
  return options;
}

async function runRenderProfile(context: ScenarioContext, arguments_: readonly string[]) {
  const options = parseOptions(arguments_);
  const atmosphere = options.mode === "atmosphere";
  const weather = options.mode === "weather";
  const world = await startWorldStack(context, {
    fixture: {
      prefix: "voxels-browser-profile-",
      source: options.worldSource,
      spawnVoxels: options.spawnVoxels,
      cascadedShadows: options.cascadedShadows,
      screenSpaceAmbientOcclusion: options.screenSpaceAmbientOcclusion,
      dayLengthSeconds:
        options.fixedDayFraction === undefined ? (atmosphere ? 48 : weather ? 0 : undefined) : 0,
      dayFractionAtUnixEpoch: options.fixedDayFraction ?? (weather ? 0.5 : undefined),
      weatherCycleSeconds:
        options.fixedWeatherFraction === undefined
          ? weather
            ? 36
            : atmosphere
              ? 0
              : undefined
          : 0,
      weatherFractionAtUnixEpoch: options.fixedWeatherFraction ?? (atmosphere ? 0.08 : undefined),
      cloudVelocityMetresPerSecond:
        options.fixedWeatherFraction === undefined && !weather ? undefined : [0, 0],
    },
    service: { metal: options.worldSource === "terrain-diffusion-30m" },
    web: { buildProfile: options.buildProfile },
  });
  const browser = await BrowserCapability.start(context, {
    warningPattern: FAILURE,
    launch: chromeWebGpuLaunchOptions(),
  });
  const navigationStarted = performance.now();
  const viewport = await browser.open({
    url: world.url,
    label: "render-profile",
    viewport: options.viewport,
    deviceScaleFactor: options.deviceScaleFactor,
    ...world.clientRoute,
  });
  const { engine, page } = viewport;
  await waitForEngine(engine);
  if (options.cameraLook !== undefined) {
    await engine.setCameraLook(options.cameraLook[0], options.cameraLook[1]);
  }
  const settledMilliseconds = performance.now() - navigationStarted;
  let traceSession = options.trace ? await startChromiumTrace(page.context(), page) : undefined;
  let traceMetrics: Record<string, number> | undefined;
  context.defer("unfinished Chromium trace", async () => {
    if (traceSession === undefined) return;
    const tracePath = context.artifacts.resolve("chromium-trace-partial.json");
    await stopChromiumTrace(traceSession, tracePath);
    context.artifacts.record("partial Chromium trace", tracePath, "application/json");
    traceSession = undefined;
  });

  let scenarios: Readonly<Record<string, unknown>>;
  if (options.mode === "stationary") {
    scenarios = {
      steady: summarizeRenderPhase(await sampleRenderSnapshots(engine, 4_000)),
    };
  } else if (options.mode === "sustained") {
    scenarios = { sustained: await sustainedProfile(engine) };
  } else if (options.mode === "directional") {
    scenarios = { directional: await sustainedProfile(engine, 2) };
  } else if (options.mode === "materials") {
    scenarios = {
      materials: await materialDetailProfile(page, engine, options.viewport.width),
    };
  } else if (options.mode === "atmosphere") {
    scenarios = { atmosphere: await atmosphereProfile(page, engine, context) };
  } else if (options.mode === "weather") {
    scenarios = { weather: await weatherProfile(page, engine, context) };
  } else {
    await mark(page, "voxels:steady:start");
    const steady = summarizeRenderPhase(await sampleRenderSnapshots(engine, 4_000));
    await mark(page, "voxels:steady:end");
    await mark(page, "voxels:traversal:start");
    await page.keyboard.down("KeyW");
    let traversalSamples: RenderSnapshotCapture[];
    try {
      traversalSamples = await sampleRenderSnapshots(engine, 6_000);
    } finally {
      await page.keyboard.up("KeyW");
    }
    const traversal = summarizeRenderPhase(traversalSamples);
    await mark(page, "voxels:traversal:end");
    scenarios = { steady, traversal };
  }
  if (options.screenshot) {
    await viewport.screenshot("render profile", { filename: "render-profile.png" });
  }
  const finalSnapshot = await engine.snapshot();

  browser.assertHealthy();
  if (traceSession !== undefined) {
    const tracePath = context.artifacts.resolve("chromium-trace.json");
    traceMetrics = await stopChromiumTrace(traceSession, tracePath);
    context.artifacts.record("Chromium trace", tracePath, "application/json");
    traceSession = undefined;
  }
  const result = {
    ok: true,
    schemaVersion: 3,
    commit: execFileSync("git", ["rev-parse", "HEAD"], { encoding: "utf8" }).trim(),
    dirty: execFileSync("git", ["status", "--porcelain"], { encoding: "utf8" }).trim() !== "",
    build: options.buildProfile,
    worldSource: options.worldSource,
    spawnVoxels: world.fixture.spawnVoxels,
    requestedLook: options.cameraLook ?? null,
    cascadedShadows: options.cascadedShadows,
    screenSpaceAmbientOcclusion: options.screenSpaceAmbientOcclusion,
    fixedDayFraction: options.fixedDayFraction ?? null,
    fixedWeatherFraction: options.fixedWeatherFraction ?? null,
    finalPose: {
      x: snapshotValue(finalSnapshot, "cameraX"),
      y: snapshotValue(finalSnapshot, "cameraY"),
      z: snapshotValue(finalSnapshot, "cameraZ"),
      yaw: snapshotValue(finalSnapshot, "yaw"),
      pitch: snapshotValue(finalSnapshot, "pitch"),
    },
    viewport: { ...options.viewport, deviceScaleFactor: options.deviceScaleFactor },
    browser: { version: browser.version },
    startup: { settledMilliseconds },
    ...scenarios,
    trace: options.trace ? { performanceMetrics: traceMetrics } : null,
    errors: 0,
  };
  await context.artifacts.writeJson("render profile report", "report.json", result);
  return {
    summary: `Render profile ${options.mode} completed.`,
    metrics: {
      settledMilliseconds,
      mode: options.mode,
    },
    details: result,
  };
}

export default defineScenario({
  id: "render-profile",
  kind: "benchmark",
  summary: "Profiles browser rendering, traversal, materials, atmosphere, weather, or endurance.",
  uses: {
    world: true,
    browser: true,
    viewport: "browser",
    screenshots: true,
    trace: true,
    metrics: true,
    rust: true,
  },
  timeoutMs: 1_800_000,
  run: runRenderProfile,
});
