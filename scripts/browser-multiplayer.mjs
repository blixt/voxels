import { execFileSync, spawn } from "node:child_process";
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { connect } from "node:net";
import { cpus, platform, release, tmpdir } from "node:os";
import path from "node:path";
import { chromium } from "playwright";
import { build, preview } from "vite-plus";
import {
  assertSnapshotSchema,
  chromeWebGpuLaunchOptions,
  isBrowserConsoleFailure,
  reserveEphemeralPort,
  SNAPSHOT,
  SNAPSHOT_SCHEMA_VERSION,
} from "./browser-harness.mjs";
import { rustTool } from "./build-wasm.ts";
import { createShapedLink } from "./network-benchmark-link.mjs";

const RESULT_SCHEMA_VERSION = 3;
const FIXTURE_VERSION = 4;
const VXWP_VERSION = 8;
const WORLD_PATH = `/v${VXWP_VERSION}/world`;
const PRESENCE_PATH = `/v${VXWP_VERSION}/presence`;
const EXPECTED_PLAYERS = 6;
const EXPECTED_REMOTE_PLAYERS = EXPECTED_PLAYERS - 1;
const BUILDER_COUNT = EXPECTED_REMOTE_PLAYERS;
const EXPECTED_PARTS_PER_AVATAR = 13;
const FAR_TIER_MINIMUM_METRES = 105;
const OBSERVER_WALK_METRES = 120;
const PLAYER_EYE_HEIGHT_METRES = 1.62;
const TOWER_MATERIAL_ID = 14;
const VIEWPORT = { width: 960, height: 540 };
const FRAME_SAMPLE_WIDTH = 5;
const FRAME_SAMPLE_START = 108;
// Six unthrottled WebGPU clients intentionally contend on one local GPU and worker pool. This gate
// catches a stall while leaving the exact p95 visible; it is not a per-device 60 fps rendering gate.
const LOCAL_MULTI_CLIENT_FRAME_P95_LIMIT_MS = 67;
const LOCAL_MULTI_CLIENT_FRAME_MAX_LIMIT_MS = 100;
const OUTPUT_DIRECTORY = path.resolve("target/multiplayer-browser");
const REQUIRE_TOWER = process.argv.includes("--require-tower");
const FAILURE =
  /panic|unreachable|runtimeerror|wgpu|webgpu|shader|sqlite|opfs|syncaccesshandle|websocket|presence|protocol|world service/i;
const LINK_PROFILE = Object.freeze({
  name: "good_remote_per_player",
  roundTripLatencyMs: 40,
  oneWayLatencyMs: 20,
  downstreamMegabitsPerSecond: 50,
  upstreamMegabitsPerSecond: 10,
  jitterMs: 0,
  packetLossPercent: 0,
});

function sleep(milliseconds) {
  return new Promise((resolve) => setTimeout(resolve, milliseconds));
}

function rounded(value, digits = 1) {
  const scale = 10 ** digits;
  return Math.round(value * scale) / scale;
}

function percentile(values, fraction) {
  if (values.length === 0) return null;
  const sorted = [...values].sort((left, right) => left - right);
  return sorted[Math.min(sorted.length - 1, Math.ceil(sorted.length * fraction) - 1)];
}

function frameTimingSummary(values) {
  const frameMs = [];
  for (let index = 0; index < values[SNAPSHOT.sampleCount]; index += 1) {
    frameMs.push(values[FRAME_SAMPLE_START + index * FRAME_SAMPLE_WIDTH]);
  }
  return {
    samples: frameMs.length,
    p95Ms: frameMs.length > 0 ? rounded(percentile(frameMs, 0.95), 3) : null,
    maxMs: frameMs.length > 0 ? rounded(Math.max(...frameMs), 3) : null,
    above33_33ms: frameMs.filter((value) => value > 33.33).length,
    droppedSamples: values[SNAPSHOT.droppedSamples],
  };
}

function observePage(page, label, errors) {
  page.on("pageerror", (error) => errors.push(`${label} pageerror: ${error.message}`));
  page.on("console", (message) => {
    if (isBrowserConsoleFailure(message.type(), message.text(), FAILURE)) {
      errors.push(`${label} ${message.type()}: ${message.text()}`);
    }
  });
}

async function snapshot(page) {
  return assertSnapshotSchema(await page.evaluate(() => globalThis.__VOXELS__.snapshot()));
}

async function aimAt(page, targetMetres) {
  const current = await snapshot(page);
  const deltaX = targetMetres.x - current[SNAPSHOT.cameraX];
  const deltaY = targetMetres.y - current[SNAPSHOT.cameraY];
  const deltaZ = targetMetres.z - current[SNAPSHOT.cameraZ];
  const horizontalDistance = Math.hypot(deltaX, deltaZ);
  const desiredYaw = Math.atan2(deltaX, -deltaZ);
  const desiredPitch = Math.atan2(deltaY, horizontalDistance);
  const yawDelta = Math.atan2(
    Math.sin(desiredYaw - current[SNAPSHOT.yaw]),
    Math.cos(desiredYaw - current[SNAPSHOT.yaw]),
  );
  await page.evaluate(
    ({ movementX, movementY }) => globalThis.__VOXELS__.look(movementX, movementY),
    {
      movementX: yawDelta / 0.0022,
      movementY: (current[SNAPSHOT.pitch] - desiredPitch) / 0.0022,
    },
  );
  await page.waitForTimeout(250);
}

async function analyzeTowerPixels(page, before, after) {
  return page.evaluate(
    async ({ beforeBase64, afterBase64 }) => {
      const pixels = async (base64) => {
        const response = await fetch(`data:image/png;base64,${base64}`);
        const image = await createImageBitmap(await response.blob());
        const canvas = new OffscreenCanvas(image.width, image.height);
        const context = canvas.getContext("2d", { willReadFrequently: true });
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
      for (let y = minY; y < maxY; y += 1) {
        for (let x = minX; x < maxX; x += 1) {
          const offset = (y * edited.width + x) * 4;
          const red = edited.values[offset];
          const green = edited.values[offset + 1];
          const blue = edited.values[offset + 2];
          const baselineRed = baseline.values[offset];
          const baselineGreen = baseline.values[offset + 1];
          const baselineBlue = baseline.values[offset + 2];
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

async function surfaceEditState(page, stride, x, z) {
  return page.evaluate(
    ({ stride: requestedStride, x: voxelX, z: voxelZ }) =>
      globalThis.__VOXELS__.surfaceEditState(requestedStride, voxelX, voxelZ),
    { stride, x, z },
  );
}

async function waitForSurfaceEditConvergence(page, stride, x, z, timeoutMs = 30_000) {
  const deadline = performance.now() + timeoutMs;
  let latest = [];
  while (performance.now() < deadline) {
    latest = await surfaceEditState(page, stride, x, z);
    if (
      latest.length === 10 &&
      latest[2] > 1 &&
      latest[3] === latest[2] &&
      latest[4] === 1 &&
      latest[5] === 0
    ) {
      return latest;
    }
    await page.waitForTimeout(50);
  }
  throw new Error(`far surface edit did not converge: ${JSON.stringify(latest)}`);
}

async function waitFor(page, label, predicate, timeoutMs = 90_000) {
  const deadline = performance.now() + timeoutMs;
  let latest = [];
  while (performance.now() < deadline) {
    latest = await snapshot(page);
    if (predicate(latest)) return latest;
    await page.waitForTimeout(50);
  }
  throw new Error(`${label} did not converge: ${JSON.stringify(latest.slice(0, 108))}`);
}

async function waitForEngine(page, label) {
  await page.waitForFunction(() => typeof globalThis.__VOXELS__?.snapshot === "function", null, {
    timeout: 30_000,
  });
  return waitFor(
    page,
    `${label} engine startup`,
    (next) =>
      next[SNAPSHOT.quads] > 0 &&
      next[SNAPSHOT.residentChunks] > 0 &&
      next[SNAPSHOT.pendingJobs] === 0,
  );
}

async function waitForRoster(player) {
  return waitFor(
    player.page,
    `${player.name} six-player roster`,
    (next) =>
      next[SNAPSHOT.remoteAvatars] === EXPECTED_REMOTE_PLAYERS &&
      next[SNAPSHOT.avatarParts] === EXPECTED_REMOTE_PLAYERS * EXPECTED_PARTS_PER_AVATAR &&
      next[SNAPSHOT.avatarDrawCalls] === EXPECTED_REMOTE_PLAYERS,
    30_000,
  );
}

async function waitForSettledWorld(player) {
  return waitFor(
    player.page,
    `${player.name} settled world coverage`,
    (next) =>
      next[SNAPSHOT.allLodsReady] === 1 &&
      next[SNAPSHOT.surfaceInFlight] === 0 &&
      next[SNAPSHOT.pendingJobs] === 0,
  );
}

function viewportFingerprint(values) {
  return [values[SNAPSHOT.viewportFingerprintLow24], values[SNAPSHOT.viewportFingerprintHigh24]];
}

async function waitForPort(port, child, logs) {
  const deadline = performance.now() + 60_000;
  while (performance.now() < deadline) {
    if (child.exitCode !== null) {
      throw new Error(`world service exited with ${child.exitCode}: ${logs.slice(-20).join("")}`);
    }
    const ready = await new Promise((resolve) => {
      const socket = connect({ host: "127.0.0.1", port });
      socket.once("connect", () => {
        socket.destroy();
        resolve(true);
      });
      socket.once("error", () => resolve(false));
    });
    if (ready) return;
    await sleep(50);
  }
  throw new Error(`world service did not listen on ${port}: ${logs.slice(-20).join("")}`);
}

async function stopChild(child) {
  if (!child || child.exitCode !== null) return;
  child.kill("SIGTERM");
  const exited = await Promise.race([
    new Promise((resolve) => child.once("exit", () => resolve(true))),
    sleep(2_000).then(() => false),
  ]);
  if (!exited && child.exitCode === null) child.kill("SIGKILL");
}

async function walkDistance(page, targetDistanceMetres) {
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
        after[SNAPSHOT.cameraX] - before[SNAPSHOT.cameraX],
        after[SNAPSHOT.cameraZ] - before[SNAPSHOT.cameraZ],
      );
      if (distance >= targetDistanceMetres) break;
    }
  } finally {
    await page.keyboard.up("KeyS");
    await page.keyboard.up("ShiftLeft");
  }
  const distanceMetres = Math.hypot(
    after[SNAPSHOT.cameraX] - before[SNAPSHOT.cameraX],
    after[SNAPSHOT.cameraZ] - before[SNAPSHOT.cameraZ],
  );
  if (distanceMetres < targetDistanceMetres) {
    throw new Error(
      `observer covered ${distanceMetres.toFixed(2)} of ${targetDistanceMetres} metres`,
    );
  }
  return { before, after, distanceMetres, durationMs: performance.now() - started };
}

function pathBytes(stats, endpoint) {
  const pathStats = stats.paths[endpoint];
  return {
    upstream: pathStats?.upstream.streamBytes ?? 0,
    downstream: pathStats?.downstream.streamBytes ?? 0,
    vxwpUpstream: pathStats?.upstream.vxwpPayloadBytes ?? 0,
    vxwpDownstream: pathStats?.downstream.vxwpPayloadBytes ?? 0,
  };
}

function playerSummary(player, current, frameTiming) {
  const network = player.link.snapshot();
  return {
    name: player.name,
    identity: player.identity,
    distanceFromSpawnMetres: rounded(
      Math.hypot(
        current[SNAPSHOT.cameraX] - player.initial[SNAPSHOT.cameraX],
        current[SNAPSHOT.cameraZ] - player.initial[SNAPSHOT.cameraZ],
      ),
      3,
    ),
    remoteAvatars: current[SNAPSHOT.remoteAvatars],
    avatarParts: current[SNAPSHOT.avatarParts],
    frameTiming,
    world: pathBytes(network, WORLD_PATH),
    presence: pathBytes(network, PRESENCE_PATH),
    messages: network.messages,
  };
}

function markdownReport(result) {
  const rows = result.players.map(
    (player) =>
      `| ${player.name} | ${player.remoteAvatars} | ${player.distanceFromSpawnMetres.toFixed(1)} | ${player.frameTiming.p95Ms.toFixed(1)} | ${player.world.downstream.toLocaleString("en-US")} | ${player.presence.upstream.toLocaleString("en-US")} | ${player.presence.downstream.toLocaleString("en-US")} |`,
  );
  return `# Six-user multiplayer browser smoke\n\nGenerated ${result.generatedAt}. Six isolated BrowserContexts used independent browser identities, OPFS state, and shaped 40 ms RTT links to one native world service.\n\n| Player | Remote avatars | Travel (m) | Frame p95 (ms) | World down (bytes) | Presence up (bytes) | Presence down (bytes) |\n| --- | ---: | ---: | ---: | ---: | ---: | ---: |\n${rows.join("\n")}\n\nAll five builders remained visible to the observer after the observer moved ${result.observer.distanceFromBuildersMetres.toFixed(1)} m away, inside the configured far presence tier.\n\nCollaborative tower gate: **${result.collaborativeTower.status}**. ${result.collaborativeTower.reason}\n`;
}

async function main() {
  const temporary = await mkdtemp(path.join(tmpdir(), "voxels-multiplayer-browser-"));
  const backendPort = await reserveEphemeralPort();
  const previewPort = await reserveEphemeralPort();
  const serviceConfigPath = path.join(temporary, "world-service.toml");
  const [serviceSource, clientSource] = await Promise.all([
    readFile("config/world-service.toml", "utf8"),
    readFile("config/client.toml", "utf8"),
  ]);
  await writeFile(
    serviceConfigPath,
    serviceSource
      .replace(/^listen = .*$/m, `listen = "127.0.0.1:${backendPort}"`)
      .replace(/^allowed_origins = .*$/m, `allowed_origins = ["http://127.0.0.1:${previewPort}"]`)
      .replace(/^database = .*$/m, 'database = "world-state-v3.sqlite3"'),
  );

  let browser;
  let previewServer;
  let worldService;
  const links = [];
  const contexts = [];
  const worldLogs = [];
  const errors = [];
  try {
    await build({ logLevel: "warn" });
    // Keep the readiness deadline about service startup, not an arbitrary clean Rust compile. This
    // also prevents a timed-out cargo child from repeatedly abandoning the same profile build.
    execFileSync(
      rustTool("cargo"),
      [
        "build",
        "--profile",
        "worldgen",
        "-p",
        "voxels-world-service",
        "--bin",
        "voxels-worldd",
        "--features",
        "terrain-metal",
      ],
      { stdio: "inherit" },
    );
    worldService = spawn(
      rustTool("cargo"),
      [
        "run",
        "--profile",
        "worldgen",
        "-p",
        "voxels-world-service",
        "--bin",
        "voxels-worldd",
        "--features",
        "terrain-metal",
        "--",
        serviceConfigPath,
      ],
      { stdio: ["ignore", "pipe", "pipe"] },
    );
    for (const stream of [worldService.stdout, worldService.stderr]) {
      stream.on("data", (bytes) => {
        worldLogs.push(bytes.toString());
        if (worldLogs.length > 200) worldLogs.shift();
      });
    }
    await waitForPort(backendPort, worldService, worldLogs);
    previewServer = await preview({
      logLevel: "warn",
      preview: { host: "127.0.0.1", port: previewPort, strictPort: true },
    });
    browser = await chromium.launch({
      ...chromeWebGpuLaunchOptions(),
      args: [
        ...chromeWebGpuLaunchOptions().args,
        "--disable-background-timer-throttling",
        "--disable-backgrounding-occluded-windows",
        "--disable-renderer-backgrounding",
      ],
    });

    const names = ["observer", "builder-1", "builder-2", "builder-3", "builder-4", "builder-5"];
    const ports = [];
    for (const _name of names) ports.push(await reserveEphemeralPort());
    for (const port of ports) {
      links.push(
        await createShapedLink({
          listenPort: port,
          targetPort: backendPort,
          profile: LINK_PROFILE,
        }),
      );
    }

    const players = await Promise.all(
      names.map(async (name, index) => {
        const context = await browser.newContext({ viewport: VIEWPORT, deviceScaleFactor: 1 });
        contexts.push(context);
        const configToml = clientSource
          .replace(/^endpoint = .*$/m, `endpoint = "ws://127.0.0.1:${ports[index]}${WORLD_PATH}"`)
          .replace(
            /^presence_endpoint = .*$/m,
            `presence_endpoint = "ws://127.0.0.1:${ports[index]}${PRESENCE_PATH}"`,
          );
        await context.route("**/config/client.toml", (route) =>
          route.fulfill({
            status: 200,
            contentType: "text/plain; charset=utf-8",
            headers: { "Cache-Control": "no-store" },
            body: configToml,
          }),
        );
        const page = await context.newPage();
        observePage(page, name, errors);
        const navigationStarted = performance.now();
        await page.goto(`http://127.0.0.1:${previewPort}/?player=${name}`, {
          waitUntil: "domcontentloaded",
        });
        const initial = await waitForEngine(page, name);
        const identity = await page.evaluate(() => globalThis.__VOXELS__.player);
        return {
          name,
          page,
          context,
          link: links[index],
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

    await mkdir(OUTPUT_DIRECTORY, { recursive: true });
    const builders = players.slice(1);
    const movementKeys = ["KeyW", "KeyS", "KeyA", "KeyD", "KeyW"];
    const builderBeforeMovement = await Promise.all(builders.map(({ page }) => snapshot(page)));
    for (let index = 0; index < builders.length; index += 1) {
      await builders[index].page.keyboard.down(movementKeys[index]);
    }
    await players[0].page.waitForTimeout(350);
    await players[0].page.screenshot({
      path: path.join(OUTPUT_DIRECTORY, "observer-near-five-walking.png"),
    });
    for (let index = 0; index < builders.length; index += 1) {
      await builders[index].page.keyboard.up(movementKeys[index]);
    }
    const builderAfterMovement = await Promise.all(builders.map(({ page }) => snapshot(page)));
    const builderTravelMetres = builders.map((_builder, index) =>
      Math.hypot(
        builderAfterMovement[index][SNAPSHOT.cameraX] -
          builderBeforeMovement[index][SNAPSHOT.cameraX],
        builderAfterMovement[index][SNAPSHOT.cameraZ] -
          builderBeforeMovement[index][SNAPSHOT.cameraZ],
      ),
    );
    if (builderTravelMetres.some((distance) => distance < 0.5)) {
      throw new Error(
        `one or more builder movement fixtures stalled: ${JSON.stringify(builderTravelMetres)}`,
      );
    }
    const observer = players[0];
    const observerWalk = await walkDistance(observer.page, OBSERVER_WALK_METRES);
    const farRosters = await Promise.all(players.map(waitForRoster));
    const builderCentroid = builders.reduce(
      (center, _builder, index) => {
        center.x += farRosters[index + 1][SNAPSHOT.cameraX] / builders.length;
        center.z += farRosters[index + 1][SNAPSHOT.cameraZ] / builders.length;
        return center;
      },
      { x: 0, z: 0 },
    );
    const observerFar = farRosters[0];
    const distanceFromBuildersMetres = Math.hypot(
      observerFar[SNAPSHOT.cameraX] - builderCentroid.x,
      observerFar[SNAPSHOT.cameraZ] - builderCentroid.z,
    );
    if (distanceFromBuildersMetres < FAR_TIER_MINIMUM_METRES) {
      throw new Error(
        `observer reached only ${distanceFromBuildersMetres.toFixed(2)}m from builders`,
      );
    }
    await aimAt(observer.page, {
      x: builderCentroid.x,
      y: observerFar[SNAPSHOT.cameraY],
      z: builderCentroid.z,
    });
    await observer.page.screenshot({ path: path.join(OUTPUT_DIRECTORY, "observer-far-five.png") });
    await observer.page.waitForTimeout(600);
    const steadySnapshots = await Promise.all(players.map(({ page }) => snapshot(page)));
    const frameTimings = steadySnapshots.map(frameTimingSummary);
    const timingViolations = frameTimings.flatMap((timing, index) => {
      const violations = [];
      if (timing.samples === 0) violations.push("captured no steady-state frames");
      if (timing.p95Ms > LOCAL_MULTI_CLIENT_FRAME_P95_LIMIT_MS) {
        violations.push(`frame p95 was ${timing.p95Ms}ms`);
      }
      if (timing.maxMs > LOCAL_MULTI_CLIENT_FRAME_MAX_LIMIT_MS) {
        violations.push(`worst frame was ${timing.maxMs}ms`);
      }
      if (timing.droppedSamples > 0) {
        violations.push(`dropped ${timing.droppedSamples} frame-history samples`);
      }
      return violations.map((violation) => `${players[index].name}: ${violation}`);
    });
    if (timingViolations.length > 0) {
      throw new Error(`steady-state frame gate failed: ${timingViolations.join(", ")}`);
    }
    if (errors.length > 0) throw new Error(errors.join("\n"));

    await Promise.all(players.map(waitForSettledWorld));
    // Four metres stays inside each ground-level builder's authoritative reach envelope. The old
    // 10 m direct-submit tower was a useful LOD stressor but also an edit-distance exploit.
    const towerVoxelCount = 40;
    const towerX = Math.floor(builderBeforeMovement[0][SNAPSHOT.cameraX] * 10);
    const towerZ = Math.floor(builderBeforeMovement[0][SNAPSHOT.cameraZ] * 10);
    const towerBaseY = Math.round(
      (builderBeforeMovement[0][SNAPSHOT.cameraY] - PLAYER_EYE_HEIGHT_METRES) * 10,
    );
    await aimAt(observer.page, {
      x: towerX / 10,
      y: (towerBaseY + towerVoxelCount / 2) / 10,
      z: towerZ / 10,
    });
    const beforeTowerScreenshot = await observer.page.screenshot({
      path: path.join(OUTPUT_DIRECTORY, "observer-far-tower-before.png"),
    });
    const beforeTowerSnapshot = await snapshot(observer.page);
    const beforeViewportFingerprint = viewportFingerprint(beforeTowerSnapshot);
    const beforeTowerNetwork = players.map((player) => player.link.snapshot());
    const beforeTowerSurfaceState = await surfaceEditState(observer.page, 16, towerX, towerZ);
    const towerStarted = performance.now();
    const submissions = await Promise.all(
      builders.flatMap((builder, builderIndex) =>
        Array.from({ length: towerVoxelCount / BUILDER_COUNT }, (_unused, localIndex) => {
          const y = towerBaseY + localIndex * BUILDER_COUNT + builderIndex;
          return builder.page.evaluate(
            ({ x, y: voxelY, z, materialId }) =>
              globalThis.__VOXELS__.submitEdit(x, voxelY, z, materialId),
            { x: towerX, y, z: towerZ, materialId: TOWER_MATERIAL_ID },
          );
        }),
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
          (next) => next[SNAPSHOT.edits] === towerVoxelCount,
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
    const renderedTower = await waitFor(
      observer.page,
      "observer rendered far tower",
      (next) => {
        const current = viewportFingerprint(next);
        return (
          current[0] !== beforeViewportFingerprint[0] || current[1] !== beforeViewportFingerprint[1]
        );
      },
      30_000,
    );
    const afterTowerScreenshot = await observer.page.screenshot({
      path: path.join(OUTPUT_DIRECTORY, "observer-far-five-tower.png"),
    });
    const visualEvidence = await analyzeTowerPixels(
      observer.page,
      beforeTowerScreenshot,
      afterTowerScreenshot,
    );
    const changedHeight = visualEvidence.changedBounds
      ? visualEvidence.changedBounds[3] - visualEvidence.changedBounds[1]
      : 0;
    if (
      visualEvidence.visiblyChangedPixels < 50 ||
      changedHeight < 20 ||
      visualEvidence.newCyanPixels < 24
    ) {
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
      const beforeWorld = pathBytes(beforeTowerNetwork[index], WORLD_PATH);
      const afterWorld = pathBytes(afterTowerNetwork[index], WORLD_PATH);
      return {
        name: player.name,
        upstream: afterWorld.upstream - beforeWorld.upstream,
        downstream: afterWorld.downstream - beforeWorld.downstream,
      };
    });
    const collaborativeTower = {
      status: "passed",
      reason: `Five builders submitted ${towerVoxelCount} authoritative voxels; every client applied them and the observer accepted the revised stride-16 surface ${distanceFromBuildersMetres.toFixed(1)} m away in ${towerConvergenceMs.toFixed(1)} ms.`,
      voxelCount: towerVoxelCount,
      heightMetres: towerVoxelCount / 10,
      builders: BUILDER_COUNT,
      distanceMetres: rounded(distanceFromBuildersMetres, 3),
      convergenceMs: rounded(towerConvergenceMs),
      observerSurface: {
        tile: observerSurfaceState.slice(0, 2),
        requiredRevision: observerSurfaceState[2],
        acceptedRevision: observerSurfaceState[3],
        resident: observerSurfaceState[4] === 1,
        dirty: observerSurfaceState[5] === 1,
        fingerprint: observerSurfaceState.slice(6, 8),
        quadCount: observerSurfaceState[8],
        activationMask: observerSurfaceState[9],
      },
      surfaceFingerprintChanged:
        beforeTowerSurfaceState[6] !== observerSurfaceState[6] ||
        beforeTowerSurfaceState[7] !== observerSurfaceState[7],
      viewportFingerprintChanged:
        renderedTower[SNAPSHOT.viewportFingerprintLow24] !== beforeViewportFingerprint[0] ||
        renderedTower[SNAPSHOT.viewportFingerprintHigh24] !== beforeViewportFingerprint[1],
      allClientsAppliedEdits: convergedClients.every(
        (next) => next[SNAPSHOT.edits] === towerVoxelCount,
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
        chrome: browser.version(),
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
        storage: "independent ephemeral BrowserContext localStorage and OPFS",
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
        ...playerSummary(player, steadySnapshots[index], frameTimings[index]),
        startupMs: rounded(player.startupMs),
      })),
      collaborativeTower,
      errors: 0,
    };
    const report = markdownReport(result);
    await Promise.all([
      writeFile(path.join(OUTPUT_DIRECTORY, "latest.json"), `${JSON.stringify(result, null, 2)}\n`),
      writeFile(path.join(OUTPUT_DIRECTORY, "latest.md"), report),
    ]);
    process.stdout.write(`${report}\nJSON: ${path.join(OUTPUT_DIRECTORY, "latest.json")}\n`);
    if (REQUIRE_TOWER && collaborativeTower.status !== "passed") {
      throw new Error(
        "collaborative tower gate was required but the current production browser/server edit path is unavailable",
      );
    }
  } finally {
    await Promise.allSettled(contexts.map((context) => context.close()));
    await browser?.close();
    await Promise.allSettled(links.map((link) => link.close()));
    await previewServer?.close();
    await stopChild(worldService);
    await rm(temporary, { recursive: true, force: true });
  }
}

await main();
