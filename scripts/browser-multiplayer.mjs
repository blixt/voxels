import { mkdir } from "node:fs/promises";
import { chromium } from "playwright";
import { build, preview } from "vite-plus";
import {
  chromeWebGpuLaunchOptions,
  isBrowserConsoleFailure,
  SNAPSHOT,
  SNAPSHOT_SCHEMA_VERSION,
} from "./browser-harness.mjs";

const FAILURE =
  /panic|unreachable|runtimeerror|wgpu|webgpu|shader|sqlite|opfs|syncaccesshandle|websocket|presence|protocol|world service/i;
const VIEWPORT = { width: 1280, height: 720 };
const PORT = 5173;
const EXPECTED_PARTS_PER_AVATAR = 13;
const SCREENSHOT_DIRECTORY = "target/multiplayer-browser";

const errors = [];
let browser;
let server;

function observePageErrors(page, player) {
  page.on("pageerror", (error) => errors.push(`${player} pageerror: ${error.message}`));
  page.on("console", (message) => {
    if (isBrowserConsoleFailure(message.type(), message.text(), FAILURE)) {
      errors.push(`${player} ${message.type()}: ${message.text()}`);
    }
  });
}

async function snapshot(page) {
  return page.evaluate(() => globalThis.__VOXELS__.snapshot());
}

async function waitFor(page, label, predicate, timeoutMs = 60_000) {
  const deadline = Date.now() + timeoutMs;
  let latest = [];
  while (Date.now() < deadline) {
    latest = await snapshot(page);
    if (predicate(latest)) return latest;
    await page.waitForTimeout(50);
  }
  throw new Error(`${label} did not converge: ${JSON.stringify(latest.slice(0, 104))}`);
}

async function waitForEngine(page, player) {
  await page.waitForFunction(() => typeof globalThis.__VOXELS__?.snapshot === "function", null, {
    timeout: 20_000,
  });
  return waitFor(
    page,
    `${player} engine startup`,
    (next) =>
      next[SNAPSHOT.schemaVersion] === SNAPSHOT_SCHEMA_VERSION &&
      next[SNAPSHOT.quads] > 0 &&
      next[SNAPSHOT.residentChunks] > 0 &&
      next[SNAPSHOT.pendingJobs] === 0,
  );
}

function avatarSummary(values) {
  return {
    remoteAvatars: values[SNAPSHOT.remoteAvatars],
    parts: values[SNAPSHOT.avatarParts],
    drawCalls: values[SNAPSHOT.avatarDrawCalls],
  };
}

function angleDelta(from, to) {
  return Math.atan2(Math.sin(to - from), Math.cos(to - from));
}

try {
  await build({ logLevel: "warn" });
  server = await preview({
    logLevel: "warn",
    // The native service intentionally allows only configured origins. Keep this harness on the
    // same canonical local origin as `vp dev` so it tests the production origin gate as well.
    preview: { host: "127.0.0.1", port: PORT, strictPort: true },
  });
  browser = await chromium.launch(chromeWebGpuLaunchOptions());
  const context = await browser.newContext({ viewport: VIEWPORT, deviceScaleFactor: 1 });
  const alice = await context.newPage();
  const bob = await context.newPage();
  observePageErrors(alice, "alice");
  observePageErrors(bob, "bob");

  await alice.goto(`http://127.0.0.1:${PORT}/?player=alice`, {
    waitUntil: "domcontentloaded",
  });
  await waitForEngine(alice, "alice");
  await bob.goto(`http://127.0.0.1:${PORT}/?player=bob`, { waitUntil: "domcontentloaded" });
  await waitForEngine(bob, "bob");

  const aliceRoster = await waitFor(
    alice,
    "alice remote avatar roster",
    (next) =>
      next[SNAPSHOT.remoteAvatars] === 1 &&
      next[SNAPSHOT.avatarParts] === EXPECTED_PARTS_PER_AVATAR,
  );
  const bobRoster = await waitFor(
    bob,
    "bob remote avatar roster",
    (next) =>
      next[SNAPSHOT.remoteAvatars] === 1 &&
      next[SNAPSHOT.avatarParts] === EXPECTED_PARTS_PER_AVATAR,
  );

  await mkdir(SCREENSHOT_DIRECTORY, { recursive: true });
  await bob.keyboard.down("KeyW");
  await bob.waitForTimeout(350);
  await alice.screenshot({ path: `${SCREENSHOT_DIRECTORY}/alice-bob-walk-a.png` });
  await bob.waitForTimeout(250);
  await alice.screenshot({ path: `${SCREENSHOT_DIRECTORY}/alice-bob-walk-b.png` });
  await bob.waitForTimeout(250);
  await bob.keyboard.up("KeyW");
  await alice.waitForTimeout(180);
  await alice.screenshot({ path: `${SCREENSHOT_DIRECTORY}/alice-bob-stopped.png` });

  const bobBeforeLook = await snapshot(bob);
  await bob.mouse.click(VIEWPORT.width / 2, VIEWPORT.height / 2);
  await bob.waitForTimeout(100);
  await bob.mouse.move(VIEWPORT.width - 80, VIEWPORT.height / 2, { steps: 16 });
  await bob.waitForTimeout(120);
  await alice.screenshot({ path: `${SCREENSHOT_DIRECTORY}/alice-bob-head-look.png` });
  await bob.waitForTimeout(850);
  await alice.screenshot({ path: `${SCREENSHOT_DIRECTORY}/alice-bob-body-follow.png` });
  const bobAfterLook = await snapshot(bob);
  const bobLookTurned = Math.abs(
    angleDelta(bobBeforeLook[SNAPSHOT.yaw], bobAfterLook[SNAPSHOT.yaw]),
  );
  if (bobLookTurned < 0.75) {
    throw new Error(`bob look turned only ${bobLookTurned.toFixed(3)} radians`);
  }

  const aliceAfterWalk = await waitFor(
    alice,
    "alice avatar after bob movement",
    (next) =>
      next[SNAPSHOT.remoteAvatars] === 1 &&
      next[SNAPSHOT.avatarParts] === EXPECTED_PARTS_PER_AVATAR,
  );
  const bobAfterWalk = await snapshot(bob);
  const bobTravelled = Math.hypot(
    bobAfterWalk[SNAPSHOT.cameraX] - bobRoster[SNAPSHOT.cameraX],
    bobAfterWalk[SNAPSHOT.cameraZ] - bobRoster[SNAPSHOT.cameraZ],
  );
  if (bobTravelled < 0.5) {
    throw new Error(`bob moved only ${bobTravelled.toFixed(3)}m during the walk fixture`);
  }
  if (errors.length > 0) throw new Error(errors.join("\n"));

  console.log(
    JSON.stringify(
      {
        ok: true,
        schemaVersion: 1,
        browserSnapshotSchema: SNAPSHOT_SCHEMA_VERSION,
        players: {
          alice: avatarSummary(aliceRoster),
          bob: avatarSummary(bobRoster),
        },
        movement: {
          bobTravelledMetres: bobTravelled,
          bobLookTurnedRadians: bobLookTurned,
          aliceAfterWalk: avatarSummary(aliceAfterWalk),
        },
        screenshots: [
          `${SCREENSHOT_DIRECTORY}/alice-bob-walk-a.png`,
          `${SCREENSHOT_DIRECTORY}/alice-bob-walk-b.png`,
          `${SCREENSHOT_DIRECTORY}/alice-bob-stopped.png`,
          `${SCREENSHOT_DIRECTORY}/alice-bob-head-look.png`,
          `${SCREENSHOT_DIRECTORY}/alice-bob-body-follow.png`,
        ],
        errors: 0,
      },
      null,
      2,
    ),
  );
} catch (error) {
  console.error(JSON.stringify({ ok: false, error: String(error), errors }, null, 2));
  process.exitCode = 1;
} finally {
  await browser?.close();
  await server?.close();
}
