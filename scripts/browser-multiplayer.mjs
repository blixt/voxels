import { spawn } from "node:child_process";
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

const RESULT_SCHEMA_VERSION = 2;
const FIXTURE_VERSION = 2;
const VXWP_VERSION = 5;
const WORLD_PATH = `/v${VXWP_VERSION}/world`;
const PRESENCE_PATH = `/v${VXWP_VERSION}/presence`;
const EXPECTED_PLAYERS = 6;
const EXPECTED_REMOTE_PLAYERS = EXPECTED_PLAYERS - 1;
const EXPECTED_PARTS_PER_AVATAR = 13;
const FAR_TIER_MINIMUM_METRES = 105;
const OBSERVER_WALK_METRES = 120;
const VIEWPORT = { width: 960, height: 540 };
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
      next[SNAPSHOT.avatarDrawCalls] === 1,
    30_000,
  );
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

function playerSummary(player, current) {
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
    frameMs: rounded(current[SNAPSHOT.frameMs], 3),
    world: pathBytes(network, WORLD_PATH),
    presence: pathBytes(network, PRESENCE_PATH),
    messages: network.messages,
  };
}

function markdownReport(result) {
  const rows = result.players.map(
    (player) =>
      `| ${player.name} | ${player.remoteAvatars} | ${player.distanceFromSpawnMetres.toFixed(1)} | ${player.world.downstream.toLocaleString("en-US")} | ${player.presence.upstream.toLocaleString("en-US")} | ${player.presence.downstream.toLocaleString("en-US")} |`,
  );
  return `# Six-user multiplayer browser smoke\n\nGenerated ${result.generatedAt}. Six isolated BrowserContexts used independent browser identities, OPFS state, and shaped 40 ms RTT links to one native world service.\n\n| Player | Remote avatars | Travel (m) | World down (bytes) | Presence up (bytes) | Presence down (bytes) |\n| --- | ---: | ---: | ---: | ---: | ---: |\n${rows.join("\n")}\n\nAll five builders remained visible to the observer after the observer moved ${result.observer.distanceFromBuildersMetres.toFixed(1)} m away, inside the configured far presence tier.\n\nCollaborative tower gate: **${result.collaborativeTower.status}**. ${result.collaborativeTower.reason}\n`;
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
      .replace(/^allowed_origins = .*$/m, `allowed_origins = ["http://127.0.0.1:${previewPort}"]`),
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
    await observer.page.screenshot({ path: path.join(OUTPUT_DIRECTORY, "observer-far-five.png") });
    if (errors.length > 0) throw new Error(errors.join("\n"));

    const collaborativeTower = {
      status: "skipped-unsupported",
      reason:
        "The browser exposes snapshot, look, and streaming-profile hooks only; VXWP v5 does not advertise SERVER_EDITS, and isolated browser profiles intentionally cannot share OPFS edits.",
      requiredBrowserDiagnosticApi: [
        "submit one voxel edit through the production server-authoritative edit path and return operation plus world revision",
        "place a deterministic test camera while marking the next presence pose as a discontinuity",
        "read a bounded voxel or surface-tile revision and content hash for convergence assertions",
      ],
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
        ...playerSummary(player, farRosters[index]),
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
    if (REQUIRE_TOWER) {
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
