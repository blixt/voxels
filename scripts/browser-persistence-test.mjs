import { chromium } from "playwright";
import { createServer as createViteServer } from "vite-plus";
import {
  assertSnapshotSchema,
  chromeWebGpuLaunchOptions,
  isBrowserConsoleFailure,
  reserveEphemeralPort,
  SNAPSHOT,
} from "./browser-harness.mjs";

const FAILURE =
  /panic|unreachable|runtimeerror|wgpu|webgpu|shader|sqlite|opfs|syncaccesshandle|nomodificationallowed|web lock request failed|no persistence leader|persistence .*failed|underwater teleport/i;
const errors = [];

function watch(name, page) {
  page.on("pageerror", (error) => errors.push(`${name} pageerror: ${error.message}`));
  page.on("console", (message) => {
    if (isBrowserConsoleFailure(message.type(), message.text(), FAILURE)) {
      errors.push(`${name} ${message.type()}: ${message.text()}`);
    }
  });
}

async function waitForEngine(page) {
  await page.waitForFunction(() => typeof globalThis.__VOXELS__?.snapshot === "function", null, {
    timeout: 20_000,
  });
  const deadline = Date.now() + 25_000;
  while (Date.now() < deadline) {
    const snapshot = await page.evaluate(() => globalThis.__VOXELS__.snapshot());
    assertSnapshotSchema(snapshot);
    if (snapshot[SNAPSHOT.quads] > 0 && snapshot[SNAPSHOT.residentChunks] > 0) return snapshot;
    await page.waitForTimeout(100);
  }
  throw new Error("engine did not render resident voxel geometry before the deadline");
}

async function assertCanvasOnly(page) {
  const body = await page.evaluate(() =>
    Array.from(document.body.children, (element) => ({
      tag: element.tagName,
      id: element.id,
      shadow: element.shadowRoot !== null,
    })),
  );
  const expected = [{ tag: "CANVAS", id: "app", shadow: false }];
  if (JSON.stringify(body) !== JSON.stringify(expected)) {
    throw new Error(`canvas-only body contract changed: ${JSON.stringify(body)}`);
  }
}

async function reloadRapidly(page, count) {
  for (let index = 0; index < count; index += 1) {
    await page.reload({ waitUntil: "commit" });
  }
  await page.waitForLoadState("domcontentloaded");
  await waitForEngine(page);
  await assertCanvasOnly(page);
}

async function reloadConcurrently(pages, rounds) {
  for (let index = 0; index < rounds; index += 1) {
    await Promise.all(pages.map((page) => page.reload({ waitUntil: "commit" })));
  }
  await Promise.all(
    pages.map(async (page) => {
      await page.waitForLoadState("domcontentloaded");
      await waitForEngine(page);
      await assertCanvasOnly(page);
    }),
  );
}

async function enterUnderwaterShowcase(page) {
  // Mission Control is canvas-rendered, so the harness deliberately exercises its stable Rust layout
  // contract rather than locating DOM controls: F3, header "more", then the second context row.
  await page.keyboard.press("F3");
  await page.waitForTimeout(100);
  await page.mouse.click(877, 90);
  await page.waitForTimeout(100);
  await page.mouse.click(789, 168);
  const deadline = Date.now() + 20_000;
  let lastSnapshot = [];
  while (Date.now() < deadline) {
    const snapshot = await page.evaluate(() => globalThis.__VOXELS__.snapshot());
    lastSnapshot = snapshot;
    assertSnapshotSchema(snapshot);
    if (
      snapshot[SNAPSHOT.immersion] > 0.5 &&
      snapshot[SNAPSHOT.eyesSubmerged] === 1 &&
      snapshot[SNAPSHOT.swimming] === 1
    ) {
      // The water-surface toggle is visual only; authoritative fluid physics must remain active.
      await page.mouse.click(902, 522);
      await page.waitForTimeout(150);
      const hiddenWater = await page.evaluate(() => globalThis.__VOXELS__.snapshot());
      assertSnapshotSchema(hiddenWater);
      if (hiddenWater[SNAPSHOT.eyesSubmerged] !== 1 || hiddenWater[SNAPSHOT.swimming] !== 1) {
        throw new Error("disabling water rendering incorrectly disabled fluid physics");
      }
      await page.mouse.click(902, 522);
      await page.keyboard.press("F3");
      await page.waitForTimeout(500);
      return snapshot;
    }
    await page.waitForTimeout(100);
  }
  throw new Error(
    `Rust underwater showcase did not enter a submerged swimming state: ${JSON.stringify(lastSnapshot.slice(0, 35))}`,
  );
}

async function closeFollowerAndAssertFinalCamera(context, url, follower) {
  const cameraFields = [
    SNAPSHOT.cameraX,
    SNAPSHOT.cameraY,
    SNAPSHOT.cameraZ,
    SNAPSHOT.yaw,
    SNAPSHOT.pitch,
  ];
  const cameraValues = (snapshot) => JSON.stringify(cameraFields.map((index) => snapshot[index]));
  // Wait until the follower has passed an idle periodic checkpoint, then change its pose and close
  // well before the next one-second save. Teardown itself must hand this final camera to the leader.
  await follower.waitForTimeout(1_200);
  const before = await follower.evaluate(() => globalThis.__VOXELS__.snapshot());
  await follower.locator("#app").click();
  await follower.waitForFunction(() => document.pointerLockElement?.id === "app");
  await follower.mouse.move(700, 160, { steps: 2 });
  const final = await follower.evaluate(() => globalThis.__VOXELS__.snapshot());
  assertSnapshotSchema(before);
  assertSnapshotSchema(final);
  if (
    Math.abs(final[SNAPSHOT.yaw] - before[SNAPSHOT.yaw]) < 0.1 ||
    Math.abs(final[SNAPSHOT.pitch] - before[SNAPSHOT.pitch]) < 0.1
  ) {
    throw new Error(
      `follower camera did not move before close: ${cameraValues(before)} -> ${cameraValues(final)}`,
    );
  }
  // Playwright's force-close may destroy a target without giving its worker a lifecycle turn.
  // Fire the real pagehide path explicitly, then allow the bounded graceful-shutdown window to
  // forward the final write and close SQLite before force-closing the now-stopped page.
  await follower.evaluate(() => {
    window.dispatchEvent(new PageTransitionEvent("pagehide", { persisted: false }));
  });
  await follower.waitForTimeout(500);
  await follower.close();

  const replacement = await context.newPage();
  watch("follower-replacement", replacement);
  await replacement.goto(url, { waitUntil: "domcontentloaded" });
  const restored = await waitForEngine(replacement);
  for (const index of cameraFields) {
    if (Math.abs(restored[index] - final[index]) > 0.0001) {
      throw new Error(
        `follower final camera was not restored at ${index}: ${cameraValues(final)} -> ${cameraValues(restored)}`,
      );
    }
  }
  return replacement;
}

const port = await reserveEphemeralPort();
const server = await createViteServer({
  server: { host: "127.0.0.1", port, strictPort: true },
});
let browser;

try {
  await server.listen();
  const address = server.httpServer?.address();
  if (!address || typeof address === "string") throw new Error("Vite did not expose a TCP port");
  const url = `http://127.0.0.1:${address.port}`;
  const leaderUrl = `${url}/?player=persistence-leader`;
  const followerUrl = `${url}/?player=persistence-follower`;

  browser = await chromium.launch(chromeWebGpuLaunchOptions());
  const context = await browser.newContext({
    viewport: { width: 960, height: 640 },
    deviceScaleFactor: 1,
  });

  const leader = await context.newPage();
  watch("leader", leader);
  await leader.goto(leaderUrl, { waitUntil: "domcontentloaded" });
  await waitForEngine(leader);
  await assertCanvasOnly(leader);

  // Model a user hammering refresh before any stable follower exists. Intermediate workers are
  // intentionally replaced before their async SQLite/WebGPU boot necessarily finishes.
  await reloadRapidly(leader, 12);

  let follower = await context.newPage();
  watch("follower", follower);
  await follower.goto(followerUrl, { waitUntil: "domcontentloaded" });
  await waitForEngine(follower);
  await assertCanvasOnly(follower);
  follower = await closeFollowerAndAssertFinalCamera(context, followerUrl, follower);
  await assertCanvasOnly(follower);

  // Reload every live browsing context together. Exactly one replacement worker must win the Web
  // Lock while the others remain usable followers; no worker may race the exclusive SAH pool.
  await reloadConcurrently([leader, follower], 4);

  // The first reload hands ownership to the follower. The remaining reloads repeatedly dispose and
  // recreate queued lock requests while that tab keeps the single OPFS lease.
  await reloadRapidly(leader, 10);

  // Closing the known owner must synchronously close SQLite, pause the SAH pool, and release the lock
  // so the surviving tab can promote without a transient OPFS error.
  await follower.close();
  await leader.waitForTimeout(250);
  await reloadRapidly(leader, 4);

  // Exercise another live follower and another owner handoff before teardown.
  const successor = await context.newPage();
  watch("successor", successor);
  await successor.goto(`${url}/?player=persistence-successor`, { waitUntil: "domcontentloaded" });
  await waitForEngine(successor);
  await leader.close();
  await successor.waitForTimeout(250);
  await reloadRapidly(successor, 4);
  await enterUnderwaterShowcase(successor);
  await assertCanvasOnly(successor);
  if (process.env.SCREENSHOT) {
    await successor.screenshot({ path: process.env.SCREENSHOT });
  }

  if (errors.length > 0) throw new Error(errors.join("\n"));
  console.log(
    JSON.stringify({
      ok: true,
      reloads: 38,
      tabs: 3,
      soloBurst: 12,
      concurrentRounds: 4,
      canvasOnly: true,
      underwater: true,
      playerKeyedCameraOnly: true,
      followerFinalCamera: true,
      persistenceErrors: 0,
    }),
  );
} catch (error) {
  console.error(JSON.stringify({ ok: false, error: String(error), errors }, null, 2));
  process.exitCode = 1;
} finally {
  await browser?.close();
  await server.close();
}
