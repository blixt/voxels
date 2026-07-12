import { chromium } from "playwright";
import { createServer as createNetServer } from "node:net";
import { createServer as createViteServer } from "vite-plus";

const FAILURE =
  /panic|unreachable|runtimeerror|wgpu|webgpu|shader|sqlite|opfs|syncaccesshandle|nomodificationallowed|web lock request failed|no persistence leader|persistence .*failed/i;
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

  if (errors.length > 0) throw new Error(errors.join("\n"));
  console.log(
    JSON.stringify({ ok: true, reloads: 18, tabs: 3, canvasOnly: true, persistenceErrors: 0 }),
  );
} catch (error) {
  console.error(JSON.stringify({ ok: false, error: String(error), errors }, null, 2));
  process.exitCode = 1;
} finally {
  await browser?.close();
  await server.close();
}
