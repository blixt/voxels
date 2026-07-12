import { chromium } from "playwright";
import { createServer as createNetServer } from "node:net";
import { createServer as createViteServer } from "vite-plus";

const FAILURE =
  /panic|unreachable|runtimeerror|wgpu|webgpu|shader|sqlite|opfs|syncaccesshandle|nomodificationallowed|web lock request failed|no persistence leader|persistence .*failed|underwater teleport/i;
const errors = [];

function watch(name, page) {
  page.on("pageerror", (error) => errors.push(`${name} pageerror: ${error.message}`));
  page.on("console", (message) => {
    if (
      (message.type() === "error" || message.type() === "warning") &&
      FAILURE.test(message.text())
    ) {
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
    if (snapshot[6] > 0 && snapshot[8] > 0) return snapshot;
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
  const expected = [
    { tag: "CANVAS", id: "app", shadow: false },
    { tag: "SCRIPT", id: "", shadow: false },
  ];
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
    if (snapshot[31] > 0.5 && snapshot[33] === 1 && snapshot[34] === 1) {
      // The water-surface toggle is visual only; authoritative fluid physics must remain active.
      await page.mouse.click(902, 522);
      await page.waitForTimeout(150);
      const hiddenWater = await page.evaluate(() => globalThis.__VOXELS__.snapshot());
      if (hiddenWater[33] !== 1 || hiddenWater[34] !== 1) {
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

async function reserveEphemeralPort() {
  const probe = createNetServer();
  await new Promise((resolve, reject) => {
    probe.once("error", reject);
    probe.listen(0, "127.0.0.1", resolve);
  });
  const address = probe.address();
  if (!address || typeof address === "string") throw new Error("could not reserve a TCP port");
  await new Promise((resolve, reject) =>
    probe.close((error) => (error ? reject(error) : resolve())),
  );
  return address.port;
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

  browser = await chromium.launch({
    channel: "chrome",
    headless: false,
    args: [
      "--headless=new",
      "--enable-unsafe-webgpu",
      "--enable-features=WebGPU",
      "--no-sandbox",
      "--hide-scrollbars",
    ],
  });
  const context = await browser.newContext({
    viewport: { width: 960, height: 640 },
    deviceScaleFactor: 1,
  });

  const leader = await context.newPage();
  watch("leader", leader);
  await leader.goto(url, { waitUntil: "domcontentloaded" });
  await waitForEngine(leader);
  await assertCanvasOnly(leader);

  const follower = await context.newPage();
  watch("follower", follower);
  await follower.goto(url, { waitUntil: "domcontentloaded" });
  await waitForEngine(follower);
  await assertCanvasOnly(follower);

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
  await successor.goto(url, { waitUntil: "domcontentloaded" });
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
      reloads: 18,
      tabs: 3,
      canvasOnly: true,
      underwater: true,
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
