import { cpus, platform, release } from "node:os";
import type { Page } from "playwright";
import {
  BrowserCapability,
  chromeWebGpuLaunchOptions,
  reserveEphemeralPort,
} from "../lib/browser.ts";
import {
  assertSnapshotSchema,
  FRAME_SAMPLE_WIDTH,
  SNAPSHOT,
  SNAPSHOT_SCHEMA_VERSION,
  snapshotValue,
} from "../lib/engine.ts";
import { createShapedLink, type LinkStats, type ShapedLink } from "../lib/network.ts";
import { PRESENCE_PATH, VXWP_VERSION, WORLD_PATH } from "../lib/protocol.ts";
import { defineScenario, type ScenarioContext } from "../lib/scenario.ts";
import {
  prepareWorldFixture,
  routeWorldClient,
  startWebPreview,
  startWorldService,
} from "../lib/world.ts";

const RESULT_SCHEMA_VERSION = 4;
const FIXTURE_VERSION = 6;
const EXPECTED_PLAYERS = 6;
const EXPECTED_REMOTE_PLAYERS = EXPECTED_PLAYERS - 1;
const BUILDER_COUNT = EXPECTED_REMOTE_PLAYERS;
const EXPECTED_PARTS_PER_AVATAR = 13;
const FAR_TIER_MINIMUM_METRES = 105;
const OBSERVER_WALK_METRES = 120;
const PLAYER_EYE_HEIGHT_METRES = 1.62;
const BUILDER_DIG_SPACING_VOXELS = 8;
const VIEWPORT = { width: 960, height: 540 };
const FRAME_SAMPLE_START = SNAPSHOT.droppedSamples + 1;
// Six unthrottled WebGPU clients intentionally contend on one local GPU and worker pool. This gate
// catches a severe stall while leaving the exact p95 visible; the far observer renders materially
// more clipmap geometry than the five near builders, so this is not a per-device frame-rate target.
const LOCAL_MULTI_CLIENT_FRAME_P95_LIMIT_MS = 150;
const LOCAL_MULTI_CLIENT_FRAME_MAX_LIMIT_MS = 250;
const FAILURE =
  /panic|unreachable|runtimeerror|wgpu|webgpu|shader|sqlite|opfs|syncaccesshandle|websocket|presence|protocol|world service/i;
const LINK_PROFILE = Object.freeze({
  name: "good_remote_per_player",
  roundTripLatencyMs: 40,
  oneWayLatencyMs: 20,
  downstreamMegabitsPerSecond: 50,
  upstreamMegabitsPerSecond: 10,
});

interface PlayerIdentity {
  readonly browserUserId: string;
  readonly playerId: string;
}

interface MultiplayerPlayer {
  readonly name: string;
  readonly page: Page;
  readonly link: ShapedLink;
  readonly initial: readonly number[];
  readonly identity: PlayerIdentity;
  readonly startupMs: number;
}

interface FrameTimingSummary {
  readonly samples: number;
  readonly p95Ms: number | null;
  readonly maxMs: number | null;
  readonly above33_33ms: number;
  readonly droppedSamples: number;
}

interface Point3 {
  readonly x: number;
  readonly y: number;
  readonly z: number;
}

function required<T>(values: readonly T[], index: number, label: string): T {
  const value = values[index];
  if (value === undefined) throw new Error(`${label} omitted index ${index}`);
  return value;
}

function rounded(value: number, digits = 1): number {
  const scale = 10 ** digits;
  return Math.round(value * scale) / scale;
}

function percentile(values: readonly number[], fraction: number): number | null {
  if (values.length === 0) return null;
  const sorted = [...values].sort((left, right) => left - right);
  return required(
    sorted,
    Math.min(sorted.length - 1, Math.ceil(sorted.length * fraction) - 1),
    "percentile",
  );
}

function frameTimingSummary(values: readonly number[]): FrameTimingSummary {
  const frameMs: number[] = [];
  for (let index = 0; index < snapshotValue(values, "sampleCount"); index += 1) {
    frameMs.push(required(values, FRAME_SAMPLE_START + index * FRAME_SAMPLE_WIDTH, "frame sample"));
  }
  const p95Ms = percentile(frameMs, 0.95);
  return {
    samples: frameMs.length,
    p95Ms: p95Ms === null ? null : rounded(p95Ms, 3),
    maxMs: frameMs.length > 0 ? rounded(Math.max(...frameMs), 3) : null,
    above33_33ms: frameMs.filter((value) => value > 33.33).length,
    droppedSamples: snapshotValue(values, "droppedSamples"),
  };
}

async function snapshot(page: Page): Promise<readonly number[]> {
  return assertSnapshotSchema(await page.evaluate(() => globalThis.__VOXELS__!.snapshot()));
}

async function aimAt(page: Page, targetMetres: Point3): Promise<void> {
  const current = await snapshot(page);
  const deltaX = targetMetres.x - snapshotValue(current, "cameraX");
  const deltaY = targetMetres.y - snapshotValue(current, "cameraY");
  const deltaZ = targetMetres.z - snapshotValue(current, "cameraZ");
  const horizontalDistance = Math.hypot(deltaX, deltaZ);
  const desiredYaw = Math.atan2(deltaX, -deltaZ);
  const desiredPitch = Math.atan2(deltaY, horizontalDistance);
  const yawDelta = Math.atan2(
    Math.sin(desiredYaw - snapshotValue(current, "yaw")),
    Math.cos(desiredYaw - snapshotValue(current, "yaw")),
  );
  await page.evaluate(
    ([movementX, movementY]) => globalThis.__VOXELS__!.look(movementX, movementY),
    [yawDelta / 0.0022, (snapshotValue(current, "pitch") - desiredPitch) / 0.0022] as const,
  );
  await page.waitForTimeout(250);
}

async function analyzeTowerPixels(page: Page, before: Buffer, after: Buffer) {
  return page.evaluate(
    async ({ beforeBase64, afterBase64 }) => {
      const pixels = async (base64: string) => {
        const response = await fetch(`data:image/png;base64,${base64}`);
        const image = await createImageBitmap(await response.blob());
        const canvas = new OffscreenCanvas(image.width, image.height);
        const context = canvas.getContext("2d", { willReadFrequently: true });
        if (context === null) throw new Error("could not create tower comparison canvas");
        context.drawImage(image, 0, 0);
        return {
          width: image.width,
          height: image.height,
          values: context.getImageData(0, 0, image.width, image.height).data,
        };
      };
      const [baseline, edited] = await Promise.all([pixels(beforeBase64), pixels(afterBase64)]);
      if (baseline.width !== edited.width || baseline.height !== edited.height) {
        throw new Error("tower screenshots changed dimensions");
      }
      const minX = Math.floor(edited.width * 0.42);
      const maxX = Math.ceil(edited.width * 0.58);
      const minY = Math.floor(edited.height * 0.28);
      const maxY = Math.ceil(edited.height * 0.72);
      let visiblyChangedPixels = 0;
      let newCyanPixels = 0;
      let maximumChannelDelta = 0;
      let changedMinX = maxX;
      let changedMinY = maxY;
      let changedMaxX = minX;
      let changedMaxY = minY;
      let cyanMinX = maxX;
      let cyanMinY = maxY;
      let cyanMaxX = minX;
      let cyanMaxY = minY;
      const channel = (values: Uint8ClampedArray, index: number): number => {
        const value = values[index];
        if (value === undefined) throw new Error(`tower image omitted channel ${index}`);
        return value;
      };
      for (let y = minY; y < maxY; y += 1) {
        for (let x = minX; x < maxX; x += 1) {
          const offset = (y * edited.width + x) * 4;
          const red = channel(edited.values, offset);
          const green = channel(edited.values, offset + 1);
          const blue = channel(edited.values, offset + 2);
          const baselineRed = channel(baseline.values, offset);
          const baselineGreen = channel(baseline.values, offset + 1);
          const baselineBlue = channel(baseline.values, offset + 2);
          const maximumDelta = Math.max(
            Math.abs(red - baselineRed),
            Math.abs(green - baselineGreen),
            Math.abs(blue - baselineBlue),
          );
          maximumChannelDelta = Math.max(maximumChannelDelta, maximumDelta);
          if (maximumDelta >= 24) {
            visiblyChangedPixels += 1;
            changedMinX = Math.min(changedMinX, x);
            changedMinY = Math.min(changedMinY, y);
            changedMaxX = Math.max(changedMaxX, x + 1);
            changedMaxY = Math.max(changedMaxY, y + 1);
          }
          const cyan = blue >= 135 && blue - red >= 35 && green - red >= 18;
          const baselineCyan =
            baselineBlue >= 135 &&
            baselineBlue - baselineRed >= 35 &&
            baselineGreen - baselineRed >= 18;
          if (cyan && !baselineCyan) {
            newCyanPixels += 1;
            cyanMinX = Math.min(cyanMinX, x);
            cyanMinY = Math.min(cyanMinY, y);
            cyanMaxX = Math.max(cyanMaxX, x + 1);
            cyanMaxY = Math.max(cyanMaxY, y + 1);
          }
        }
      }
      return {
        region: [minX, minY, maxX, maxY],
        visiblyChangedPixels,
        newCyanPixels,
        maximumChannelDelta,
        changedBounds:
          visiblyChangedPixels > 0 ? [changedMinX, changedMinY, changedMaxX, changedMaxY] : null,
        newCyanBounds: newCyanPixels > 0 ? [cyanMinX, cyanMinY, cyanMaxX, cyanMaxY] : null,
      };
    },
    { beforeBase64: before.toString("base64"), afterBase64: after.toString("base64") },
  );
}

async function surfaceEditState(
  page: Page,
  stride: number,
  x: number,
  z: number,
): Promise<readonly number[]> {
  return page.evaluate(
    ([requestedStride, voxelX, voxelZ]) =>
      globalThis.__VOXELS__!.surfaceEditState(requestedStride, voxelX, voxelZ),
    [stride, x, z] as const,
  );
}

async function inventory(page: Page): Promise<readonly number[]> {
  return page.evaluate(() => globalThis.__VOXELS__!.inventory());
}

async function waitForEarnedInventory(
  page: Page,
  label: string,
  previousRevision: number,
  timeoutMs = 30_000,
): Promise<readonly number[]> {
  const deadline = performance.now() + timeoutMs;
  let latest: readonly number[] = [];
  while (performance.now() < deadline) {
    latest = await inventory(page);
    if (
      required(latest, 0, "inventory revision") > previousRevision &&
      latest.slice(2).some((count) => count > 0)
    ) {
      return latest;
    }
    await page.waitForTimeout(50);
  }
  throw new Error(`${label} did not receive earned inventory: ${JSON.stringify(latest)}`);
}

async function waitForSurfaceEditConvergence(
  page: Page,
  stride: number,
  x: number,
  z: number,
  timeoutMs = 30_000,
): Promise<readonly number[]> {
  const deadline = performance.now() + timeoutMs;
  let latest: readonly number[] = [];
  while (performance.now() < deadline) {
    latest = await surfaceEditState(page, stride, x, z);
    if (
      latest.length === 10 &&
      required(latest, 2, "surface required revision") > 1 &&
      required(latest, 3, "surface accepted revision") ===
        required(latest, 2, "surface required revision") &&
      required(latest, 4, "surface resident") === 1 &&
      required(latest, 5, "surface dirty") === 0
    ) {
      return latest;
    }
    await page.waitForTimeout(50);
  }
  throw new Error(`far surface edit did not converge: ${JSON.stringify(latest)}`);
}

async function waitFor(
  page: Page,
  label: string,
  predicate: (snapshot: readonly number[]) => boolean,
  timeoutMs = 90_000,
): Promise<readonly number[]> {
  const deadline = performance.now() + timeoutMs;
  let latest: readonly number[] = [];
  while (performance.now() < deadline) {
    latest = await snapshot(page);
    if (predicate(latest)) return latest;
    await page.waitForTimeout(50);
  }
  throw new Error(`${label} did not converge: ${JSON.stringify(latest.slice(0, 108))}`);
}

async function waitForEngine(page: Page, label: string): Promise<readonly number[]> {
  await page.waitForFunction(() => typeof globalThis.__VOXELS__?.snapshot === "function", null, {
    timeout: 30_000,
  });
  return waitFor(
    page,
    `${label} engine startup`,
    (next) =>
      snapshotValue(next, "quads") > 0 &&
      snapshotValue(next, "residentChunks") > 0 &&
      snapshotValue(next, "pendingJobs") === 0,
  );
}

async function waitForRoster(player: MultiplayerPlayer): Promise<readonly number[]> {
  return waitFor(
    player.page,
    `${player.name} six-player roster`,
    (next) =>
      snapshotValue(next, "remoteAvatars") === EXPECTED_REMOTE_PLAYERS &&
      // Draw calls are view-dependent because the renderer culls complete off-screen avatars.
      // Roster membership and retained body parts are the provider-neutral connection invariant.
      snapshotValue(next, "avatarParts") === EXPECTED_REMOTE_PLAYERS * EXPECTED_PARTS_PER_AVATAR,
    30_000,
  );
}

async function waitForSettledWorld(player: MultiplayerPlayer): Promise<readonly number[]> {
  return waitFor(
    player.page,
    `${player.name} settled world coverage`,
    (next) =>
      snapshotValue(next, "allLodsReady") === 1 &&
      snapshotValue(next, "surfaceInFlight") === 0 &&
      snapshotValue(next, "pendingJobs") === 0,
  );
}

function viewportFingerprint(values: readonly number[]): readonly [number, number] {
  return [
    snapshotValue(values, "viewportFingerprintLow24"),
    snapshotValue(values, "viewportFingerprintHigh24"),
  ];
}

async function walkDistance(
  page: Page,
  targetDistanceMetres: number,
): Promise<{
  readonly before: readonly number[];
  readonly after: readonly number[];
  readonly distanceMetres: number;
  readonly durationMs: number;
}> {
  const before = await snapshot(page);
  const started = performance.now();
  await page.keyboard.down("ShiftLeft");
  await page.keyboard.down("KeyS");
  let after = before;
  try {
    while (performance.now() - started < 35_000) {
      await page.waitForTimeout(50);
      after = await snapshot(page);
      const distance = Math.hypot(
        snapshotValue(after, "cameraX") - snapshotValue(before, "cameraX"),
        snapshotValue(after, "cameraZ") - snapshotValue(before, "cameraZ"),
      );
      if (distance >= targetDistanceMetres) break;
    }
  } finally {
    await page.keyboard.up("KeyS");
    await page.keyboard.up("ShiftLeft");
  }
  const distanceMetres = Math.hypot(
    snapshotValue(after, "cameraX") - snapshotValue(before, "cameraX"),
    snapshotValue(after, "cameraZ") - snapshotValue(before, "cameraZ"),
  );
  if (distanceMetres < targetDistanceMetres) {
    throw new Error(
      `observer covered ${distanceMetres.toFixed(2)} of ${targetDistanceMetres} metres`,
    );
  }
  return { before, after, distanceMetres, durationMs: performance.now() - started };
}

function pathBytes(stats: LinkStats, endpoint: string) {
  const pathStats = stats.paths[endpoint];
  return {
    upstream: pathStats?.upstream.streamBytes ?? 0,
    downstream: pathStats?.downstream.streamBytes ?? 0,
    vxwpUpstream: pathStats?.upstream.vxwpPayloadBytes ?? 0,
    vxwpDownstream: pathStats?.downstream.vxwpPayloadBytes ?? 0,
  };
}

function playerSummary(
  player: MultiplayerPlayer,
  current: readonly number[],
  frameTiming: FrameTimingSummary,
) {
  const network = player.link.snapshot();
  return {
    name: player.name,
    identity: player.identity,
    distanceFromSpawnMetres: rounded(
      Math.hypot(
        snapshotValue(current, "cameraX") - snapshotValue(player.initial, "cameraX"),
        snapshotValue(current, "cameraZ") - snapshotValue(player.initial, "cameraZ"),
      ),
      3,
    ),
    remoteAvatars: snapshotValue(current, "remoteAvatars"),
    avatarParts: snapshotValue(current, "avatarParts"),
    frameTiming,
    world: pathBytes(network, WORLD_PATH),
    presence: pathBytes(network, PRESENCE_PATH),
    messages: network.messages,
  };
}

interface MultiplayerReportPlayer {
  readonly name: string;
  readonly remoteAvatars: number;
  readonly distanceFromSpawnMetres: number;
  readonly frameTiming: FrameTimingSummary;
  readonly world: ReturnType<typeof pathBytes>;
  readonly presence: ReturnType<typeof pathBytes>;
}

interface MultiplayerReport {
  readonly generatedAt: string;
  readonly players: readonly MultiplayerReportPlayer[];
  readonly observer: { readonly distanceFromBuildersMetres: number };
  readonly collaborativeTower: { readonly status: string; readonly reason: string };
}

function markdownReport(result: MultiplayerReport): string {
  const rows = result.players.map((player) => {
    const p95 = player.frameTiming.p95Ms;
    return `| ${player.name} | ${player.remoteAvatars} | ${player.distanceFromSpawnMetres.toFixed(1)} | ${p95 === null ? "n/a" : p95.toFixed(1)} | ${player.world.downstream.toLocaleString("en-US")} | ${player.presence.upstream.toLocaleString("en-US")} | ${player.presence.downstream.toLocaleString("en-US")} |`;
  });
  return `# Six-user multiplayer browser smoke\n\nGenerated ${result.generatedAt}. Six isolated BrowserContexts used independent browser identities and shaped 40 ms RTT links to one native world service.\n\n| Player | Remote avatars | Travel (m) | Frame p95 (ms) | World down (bytes) | Presence up (bytes) | Presence down (bytes) |\n| --- | ---: | ---: | ---: | ---: | ---: | ---: |\n${rows.join("\n")}\n\nAll five builders remained visible to the observer after the observer moved ${result.observer.distanceFromBuildersMetres.toFixed(1)} m away, inside the configured far presence tier.\n\nCollaborative tower gate: **${result.collaborativeTower.status}**. ${result.collaborativeTower.reason}\n`;
}

async function main(scenario: ScenarioContext, arguments_: readonly string[]) {
  if (arguments_.length > 0) {
    throw new Error(`multiplayer takes no arguments; received ${arguments_.join(" ")}`);
  }
  const previewPort = await reserveEphemeralPort();
  const names = ["observer", "builder-1", "builder-2", "builder-3", "builder-4", "builder-5"];
  const ports = await Promise.all(names.map(() => reserveEphemeralPort()));
  const fixture = await prepareWorldFixture({
    originPort: previewPort,
    clientPorts: ports,
    prefix: "voxels-multiplayer-browser-",
    source: "terrain-diffusion-30m",
  });
  scenario.defer("multiplayer fixture", () => fixture.cleanup());
  await startWebPreview(scenario, { port: previewPort, buildProfile: "release" });
  const service = await startWorldService(scenario, fixture, { metal: true });
  const links = await Promise.all(
    ports.map((port) =>
      createShapedLink({
        listenPort: port,
        targetPort: fixture.backendPort,
        profile: LINK_PROFILE,
      }),
    ),
  );
  for (const [index, link] of links.entries()) {
    scenario.defer(`multiplayer shaped link ${index + 1}`, () => link.close());
  }
  const launch = chromeWebGpuLaunchOptions();
  const browser = await BrowserCapability.start(scenario, {
    warningPattern: FAILURE,
    launch: {
      ...launch,
      args: [
        ...(launch.args ?? []),
        "--disable-background-timer-throttling",
        "--disable-backgrounding-occluded-windows",
        "--disable-renderer-backgrounding",
      ],
    },
  });
  try {
    const players: MultiplayerPlayer[] = await Promise.all(
      names.map(async (name, index) => {
        const navigationStarted = performance.now();
        const viewport = await browser.open({
          url: `http://127.0.0.1:${previewPort}/?player=${name}`,
          label: name,
          viewport: VIEWPORT,
          deviceScaleFactor: 1,
          engine: false,
          ...routeWorldClient(fixture, index),
        });
        const page = viewport.page;
        const initial = await waitForEngine(page, name);
        const identity = await page.evaluate(() => globalThis.__VOXELS__!.player);
        return {
          name,
          page,
          link: required(links, index, `${name} shaped link`),
          initial,
          identity,
          startupMs: performance.now() - navigationStarted,
        };
      }),
    );

    const browserUserIds = new Set(players.map((player) => player.identity.browserUserId));
    const playerIds = new Set(players.map((player) => player.identity.playerId));
    if (browserUserIds.size !== EXPECTED_PLAYERS || playerIds.size !== EXPECTED_PLAYERS) {
      throw new Error(
        `isolated contexts reused identities: browser users=${browserUserIds.size}, players=${playerIds.size}`,
      );
    }

    const rosterStarted = performance.now();
    await Promise.all(players.map(waitForRoster));
    const initialRosterMs = performance.now() - rosterStarted;

    const builders = players.slice(1);
    const observer = required(players, 0, "observer");
    // Keep the builders together while moving beyond the protected 6.4 m starting area. The later
    // dig and tower remain ordinary reach-checked player actions at this editable worksite.
    const builderWalks = builders.map(({ page }) => walkDistance(page, 10));
    await observer.page.waitForTimeout(350);
    const walkingScreenshot = scenario.artifacts.resolve("observer-near-five-walking.png");
    await observer.page.screenshot({ path: walkingScreenshot });
    scenario.artifacts.record("near walking builders", walkingScreenshot, "image/png");
    const completedBuilderWalks = await Promise.all(builderWalks);
    const builderTravelMetres = completedBuilderWalks.map((walk) => walk.distanceMetres);
    const builderAfterMovement = await Promise.all(
      builders.map((builder) =>
        waitFor(
          builder.page,
          `${builder.name} grounded worksite`,
          (next) => snapshotValue(next, "grounded") === 1,
          30_000,
        ),
      ),
    );
    const observerWalk = await walkDistance(observer.page, OBSERVER_WALK_METRES);
    const farRosters = await Promise.all(players.map(waitForRoster));
    const builderCentroid = builders.reduce(
      (center, _builder, index) => {
        const roster = required(farRosters, index + 1, "far builder roster");
        center.x += snapshotValue(roster, "cameraX") / builders.length;
        center.z += snapshotValue(roster, "cameraZ") / builders.length;
        return center;
      },
      { x: 0, z: 0 },
    );
    const observerFar = required(farRosters, 0, "far observer roster");
    const distanceFromBuildersMetres = Math.hypot(
      snapshotValue(observerFar, "cameraX") - builderCentroid.x,
      snapshotValue(observerFar, "cameraZ") - builderCentroid.z,
    );
    if (distanceFromBuildersMetres < FAR_TIER_MINIMUM_METRES) {
      throw new Error(
        `observer reached only ${distanceFromBuildersMetres.toFixed(2)}m from builders`,
      );
    }
    await aimAt(observer.page, {
      x: builderCentroid.x,
      y: snapshotValue(observerFar, "cameraY"),
      z: builderCentroid.z,
    });
    const farScreenshot = scenario.artifacts.resolve("observer-far-five.png");
    await observer.page.screenshot({ path: farScreenshot });
    scenario.artifacts.record("far builders", farScreenshot, "image/png");
    await Promise.all(players.map(waitForSettledWorld));
    // Drain unequal startup/walk histories, then measure one identical steady window everywhere.
    await Promise.all(players.map(({ page }) => snapshot(page)));
    await observer.page.waitForTimeout(3_000);
    const steadySnapshots = await Promise.all(players.map(({ page }) => snapshot(page)));
    const frameTimings = steadySnapshots.map(frameTimingSummary);
    const timingViolations = frameTimings.flatMap((timing, index) => {
      const violations: string[] = [];
      if (timing.samples === 0) violations.push("captured no steady-state frames");
      if (timing.p95Ms !== null && timing.p95Ms > LOCAL_MULTI_CLIENT_FRAME_P95_LIMIT_MS) {
        violations.push(`frame p95 was ${timing.p95Ms}ms`);
      }
      if (timing.maxMs !== null && timing.maxMs > LOCAL_MULTI_CLIENT_FRAME_MAX_LIMIT_MS) {
        violations.push(`worst frame was ${timing.maxMs}ms`);
      }
      if (timing.droppedSamples > 0) {
        violations.push(`dropped ${timing.droppedSamples} frame-history samples`);
      }
      const player = required(players, index, "timing player");
      return violations.map((violation) => `${player.name}: ${violation}`);
    });
    if (timingViolations.length > 0) {
      const diagnostics = players.map((player, index) => ({
        name: player.name,
        timing: required(frameTimings, index, "frame timing"),
        quads: snapshotValue(required(steadySnapshots, index, "steady snapshot"), "quads"),
        drawCalls: snapshotValue(required(steadySnapshots, index, "steady snapshot"), "drawCalls"),
        arenaAllocatedMiB: snapshotValue(
          required(steadySnapshots, index, "steady snapshot"),
          "arenaAllocatedMiB",
        ),
      }));
      throw new Error(
        `steady-state frame gate failed: ${timingViolations.join(", ")}; ${JSON.stringify(diagnostics)}`,
      );
    }
    if (browser.failures.length > 0) {
      throw new Error(
        browser.failures
          .map((error) => `${error.page} ${error.source}: ${error.message}`)
          .join("\n"),
      );
    }

    const inventoryBeforeDig = await Promise.all(builders.map(({ page }) => inventory(page)));
    const digSubmissions = await Promise.all(
      builders.map((builder, index) => {
        const position = required(builderAfterMovement, index, "builder dig position");
        const lateralOffset = (index - Math.floor(BUILDER_COUNT / 2)) * BUILDER_DIG_SPACING_VOXELS;
        return builder.page.evaluate(([x, y, z]) => globalThis.__VOXELS__!.submitDig(x, y, z), [
          Math.floor(snapshotValue(position, "cameraX") * 10) + lateralOffset,
          Math.round((snapshotValue(position, "cameraY") - PLAYER_EYE_HEIGHT_METRES) * 10 - 1),
          Math.floor(snapshotValue(position, "cameraZ") * 10),
        ] as const);
      }),
    );
    if (digSubmissions.some((submitted) => !submitted)) {
      throw new Error("one or more production dig submissions were backpressured or rejected");
    }
    const earnedInventories = await Promise.all(
      builders.map((builder, index) =>
        waitForEarnedInventory(
          builder.page,
          `${builder.name} dig`,
          required(
            required(inventoryBeforeDig, index, "pre-dig inventory"),
            0,
            "inventory revision",
          ),
        ),
      ),
    );

    // A 4.5 m column with a 1.7 m crossbar remains inside every builder's authoritative reach
    // envelope while producing a legible far-LOD silhouette from ordinary mined material.
    const builderOrigin = required(builderAfterMovement, 0, "tower builder origin");
    const towerX = Math.floor(snapshotValue(builderOrigin, "cameraX") * 10);
    const towerZ = Math.floor(snapshotValue(builderOrigin, "cameraZ") * 10);
    const towerBaseY = Math.round(
      (snapshotValue(builderOrigin, "cameraY") - PLAYER_EYE_HEIGHT_METRES) * 10,
    );
    const towerHeightVoxels = 45;
    const towerTopY = towerBaseY + towerHeightVoxels - 1;
    const towerVoxels = Array.from({ length: towerHeightVoxels }, (_unused, offset) => ({
      x: towerX,
      y: towerBaseY + offset,
      z: towerZ,
    }));
    for (let offset = -8; offset <= 8; offset += 1) {
      if (offset === 0) continue;
      towerVoxels.push({ x: towerX + offset, y: towerTopY, z: towerZ });
      towerVoxels.push({ x: towerX, y: towerTopY, z: towerZ + offset });
    }
    const towerVoxelCount = towerVoxels.length;
    const placementsPerBuilder = Math.ceil(towerVoxelCount / BUILDER_COUNT);
    const firstEarnedInventory = required(earnedInventories, 0, "earned inventory");
    const towerMaterialId = Array.from(
      { length: firstEarnedInventory.length - 2 },
      (_unused, index) => index + 1,
    )
      .filter((materialId) =>
        earnedInventories.every(
          (earnedInventory) =>
            required(earnedInventory, materialId + 1, "material inventory") >= placementsPerBuilder,
        ),
      )
      .sort(
        (left, right) =>
          Math.min(
            ...earnedInventories.map((values) => required(values, right + 1, "material inventory")),
          ) -
          Math.min(
            ...earnedInventories.map((values) => required(values, left + 1, "material inventory")),
          ),
      )[0];
    if (towerMaterialId === undefined) {
      throw new Error(
        `builders did not dig enough of one shared material: ${JSON.stringify(earnedInventories)}`,
      );
    }
    const dugVoxelCount = earnedInventories.reduce(
      (total, values) => total + values.slice(2).reduce((sum, count) => sum + count, 0),
      0,
    );
    await Promise.all(
      players.map((player) =>
        waitFor(
          player.page,
          `${player.name} authoritative collaborative digs`,
          (next) => snapshotValue(next, "edits") === dugVoxelCount,
          30_000,
        ),
      ),
    );
    await aimAt(observer.page, {
      x: towerX / 10,
      y: (towerBaseY + towerHeightVoxels / 2) / 10,
      z: towerZ / 10,
    });
    const beforeTowerPath = scenario.artifacts.resolve("observer-far-tower-before.png");
    const beforeTowerScreenshot = await observer.page.screenshot({ path: beforeTowerPath });
    scenario.artifacts.record("tower before", beforeTowerPath, "image/png");
    const beforeTowerSnapshot = await snapshot(observer.page);
    const beforeViewportFingerprint = viewportFingerprint(beforeTowerSnapshot);
    const beforeTowerNetwork = players.map((player) => player.link.snapshot());
    const beforeTowerSurfaceState = await surfaceEditState(observer.page, 16, towerX, towerZ);
    const towerStarted = performance.now();
    const submissions = await Promise.all(
      builders.flatMap((builder, builderIndex) =>
        towerVoxels
          .filter((_voxel, index) => index % BUILDER_COUNT === builderIndex)
          .map((voxel) =>
            builder.page.evaluate(
              ([x, y, z, materialId]) => globalThis.__VOXELS__!.submitEdit(x, y, z, materialId),
              [voxel.x, voxel.y, voxel.z, towerMaterialId] as const,
            ),
          ),
      ),
    );
    if (submissions.some((submitted) => !submitted)) {
      throw new Error("one or more production edit submissions were backpressured or rejected");
    }
    const convergedClients = await Promise.all(
      players.map((player) =>
        waitFor(
          player.page,
          `${player.name} authoritative tower edits`,
          (next) => snapshotValue(next, "edits") === dugVoxelCount + towerVoxelCount,
          30_000,
        ),
      ),
    );
    const observerSurfaceState = await waitForSurfaceEditConvergence(
      observer.page,
      16,
      towerX,
      towerZ,
    );
    const towerConvergenceMs = performance.now() - towerStarted;
    await observer.page.waitForTimeout(1_000);
    const afterTowerPath = scenario.artifacts.resolve("observer-far-five-tower.png");
    const afterTowerScreenshot = await observer.page.screenshot({ path: afterTowerPath });
    scenario.artifacts.record("tower after", afterTowerPath, "image/png");
    const afterTowerSnapshot = await snapshot(observer.page);
    const visualEvidence = await analyzeTowerPixels(
      observer.page,
      beforeTowerScreenshot,
      afterTowerScreenshot,
    );
    const changedHeight = visualEvidence.changedBounds
      ? required(visualEvidence.changedBounds, 3, "changed bounds") -
        required(visualEvidence.changedBounds, 1, "changed bounds")
      : 0;
    if (visualEvidence.visiblyChangedPixels < 50 || changedHeight < 8) {
      throw new Error(
        `distant tower was not visually legible: ${JSON.stringify({
          visualEvidence,
          beforeSurface: beforeTowerSurfaceState,
          afterSurface: observerSurfaceState,
        })}`,
      );
    }
    const afterTowerNetwork = players.map((player) => player.link.snapshot());
    const towerTraffic = players.map((player, index) => {
      const beforeWorld = pathBytes(
        required(beforeTowerNetwork, index, "pre-tower network"),
        WORLD_PATH,
      );
      const afterWorld = pathBytes(
        required(afterTowerNetwork, index, "post-tower network"),
        WORLD_PATH,
      );
      return {
        name: player.name,
        upstream: afterWorld.upstream - beforeWorld.upstream,
        downstream: afterWorld.downstream - beforeWorld.downstream,
      };
    });
    const collaborativeTower = {
      status: "passed",
      reason: `Five builders dug their own material, placed ${towerVoxelCount} authoritative voxels, and every client applied them; the observer accepted the revised stride-16 surface ${distanceFromBuildersMetres.toFixed(1)} m away in ${towerConvergenceMs.toFixed(1)} ms.`,
      voxelCount: towerVoxelCount,
      materialId: towerMaterialId,
      dugVoxelCount,
      heightMetres: towerHeightVoxels / 10,
      builders: BUILDER_COUNT,
      distanceMetres: rounded(distanceFromBuildersMetres, 3),
      convergenceMs: rounded(towerConvergenceMs),
      observerSurface: {
        tile: observerSurfaceState.slice(0, 2),
        requiredRevision: required(observerSurfaceState, 2, "surface required revision"),
        acceptedRevision: required(observerSurfaceState, 3, "surface accepted revision"),
        resident: required(observerSurfaceState, 4, "surface resident") === 1,
        dirty: required(observerSurfaceState, 5, "surface dirty") === 1,
        fingerprint: observerSurfaceState.slice(6, 8),
        quadCount: required(observerSurfaceState, 8, "surface quad count"),
        activationMask: required(observerSurfaceState, 9, "surface activation mask"),
      },
      surfaceFingerprintChanged:
        required(beforeTowerSurfaceState, 6, "surface fingerprint") !==
          required(observerSurfaceState, 6, "surface fingerprint") ||
        required(beforeTowerSurfaceState, 7, "surface fingerprint") !==
          required(observerSurfaceState, 7, "surface fingerprint"),
      viewportFingerprintChanged:
        snapshotValue(afterTowerSnapshot, "viewportFingerprintLow24") !==
          beforeViewportFingerprint[0] ||
        snapshotValue(afterTowerSnapshot, "viewportFingerprintHigh24") !==
          beforeViewportFingerprint[1],
      allClientsAppliedEdits: convergedClients.every(
        (next) => snapshotValue(next, "edits") === dugVoxelCount + towerVoxelCount,
      ),
      visualEvidence,
      traffic: towerTraffic,
    };
    const result = {
      schemaVersion: RESULT_SCHEMA_VERSION,
      generatedAt: new Date().toISOString(),
      environment: {
        platform: `${platform()} ${release()}`,
        cpu: cpus()[0]?.model ?? "unknown",
        logicalCpus: cpus().length,
        chrome: browser.version,
        node: process.version,
      },
      fixture: {
        version: FIXTURE_VERSION,
        browserContexts: EXPECTED_PLAYERS,
        browserSnapshotSchema: SNAPSHOT_SCHEMA_VERSION,
        protocol: { name: "VXWP", version: VXWP_VERSION },
        link: LINK_PROFILE,
        observerWalkMetres: OBSERVER_WALK_METRES,
        farTierMinimumMetres: FAR_TIER_MINIMUM_METRES,
        localMultiClientFrameP95LimitMs: LOCAL_MULTI_CLIENT_FRAME_P95_LIMIT_MS,
        localMultiClientFrameMaxLimitMs: LOCAL_MULTI_CLIENT_FRAME_MAX_LIMIT_MS,
        storage: "independent ephemeral BrowserContext localStorage",
      },
      initialRosterMs: rounded(initialRosterMs),
      observer: {
        walkDurationMs: rounded(observerWalk.durationMs),
        walkDistanceMetres: rounded(observerWalk.distanceMetres, 3),
        distanceFromBuildersMetres: rounded(distanceFromBuildersMetres, 3),
      },
      builders: {
        travelMetres: builderTravelMetres.map((distance) => rounded(distance, 3)),
      },
      players: players.map((player, index) => ({
        ...playerSummary(
          player,
          required(steadySnapshots, index, "steady snapshot"),
          required(frameTimings, index, "frame timing"),
        ),
        startupMs: rounded(player.startupMs),
      })),
      collaborativeTower,
      errors: 0,
    };
    const report = markdownReport(result);
    await Promise.all([
      scenario.artifacts.writeJson("multiplayer report", "report.json", result),
      scenario.artifacts.writeText("multiplayer report", "report.md", report, "text/markdown"),
    ]);
    return {
      summary: "Six clients and the collaborative far-distance tower converged.",
      metrics: {
        initialRosterMs: result.initialRosterMs,
        towerConvergenceMs: collaborativeTower.convergenceMs,
        distanceFromBuildersMetres: result.observer.distanceFromBuildersMetres,
      },
      details: result,
    };
  } catch (error) {
    const reason = error instanceof Error ? error.stack : String(error);
    const serviceLog = service.logs.join("").trim();
    throw new Error(
      `${reason}\n\nNative world-service output:\n${serviceLog || "(no output captured)"}`,
      { cause: error },
    );
  }
}

export default defineScenario({
  id: "multiplayer",
  kind: "validation",
  summary: "Validates six isolated browser clients, presence, edits, and far-LOD convergence.",
  uses: {
    world: true,
    browser: true,
    viewport: "browser",
    screenshots: true,
    network: true,
    metrics: true,
    rust: true,
  },
  timeoutMs: 1_800_000,
  run: main,
});
