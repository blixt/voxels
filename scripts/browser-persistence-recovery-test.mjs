import { chromium } from "playwright";
import { createServer as createViteServer } from "vite-plus";
import {
  assertSnapshotSchema,
  chromeWebGpuLaunchOptions,
  reserveEphemeralPort,
  SNAPSHOT,
} from "./browser-harness.mjs";

const FIRST_OPEN_ATTEMPTS = 20;
const failures = [];

const port = await reserveEphemeralPort();
const server = await createViteServer({
  server: { host: "127.0.0.1", port, strictPort: true },
});
let browser;

try {
  await server.listen();
  const address = server.httpServer?.address();
  if (!address || typeof address === "string") throw new Error("Vite did not expose a TCP port");

  browser = await chromium.launch(chromeWebGpuLaunchOptions());
  const context = await browser.newContext({ viewport: { width: 960, height: 640 } });
  const page = await context.newPage();
  let appWorker;
  let injectedWorkers = 0;

  page.on("pageerror", (error) => failures.push(`pageerror: ${error.message}`));
  page.on("console", (message) => {
    if (message.type() === "error" || message.type() === "warning") {
      failures.push(`${message.type()}: ${message.text()}`);
    }
  });
  page.on("worker", (worker) => {
    appWorker = worker;
  });
  await page.route("**/web/worker.ts*", async (route) => {
    const response = await route.fetch();
    const source = await response.text();
    const setup = `
      const storage = navigator.storage;
      const getDirectory = storage.getDirectory.bind(storage);
      const probe = { calls: 0, failures: 0 };
      globalThis.__voxelsOpfsRecoveryProbe = probe;
      Object.defineProperty(storage, "getDirectory", {
        configurable: true,
        value: async () => {
          probe.calls += 1;
          if (probe.failures < ${FIRST_OPEN_ATTEMPTS}) {
            probe.failures += 1;
            throw new DOMException("injected stale OPFS lease", "NoModificationAllowedError");
          }
          return getDirectory();
        },
      });
    `;
    injectedWorkers += 1;
    await route.fulfill({ response, body: setup + source });
  });

  await page.goto(`http://127.0.0.1:${address.port}`, { waitUntil: "domcontentloaded" });
  await page.waitForFunction(() => typeof globalThis.__VOXELS__?.snapshot === "function", null, {
    timeout: 20_000,
  });
  const deadline = Date.now() + 25_000;
  let rendered = false;
  while (Date.now() < deadline) {
    const snapshot = await page.evaluate(() => globalThis.__VOXELS__.snapshot());
    assertSnapshotSchema(snapshot);
    if (snapshot[SNAPSHOT.quads] > 0) {
      rendered = true;
      break;
    }
    await page.waitForTimeout(100);
  }
  if (!rendered)
    throw new Error("engine did not render resident voxel geometry before the deadline");
  const probe = await appWorker?.evaluate(() => globalThis.__voxelsOpfsRecoveryProbe);
  const body = await page.evaluate(() =>
    Array.from(document.body.children, (node) => node.tagName),
  );
  const recovered =
    injectedWorkers === 1 &&
    probe?.failures === FIRST_OPEN_ATTEMPTS &&
    probe.calls > FIRST_OPEN_ATTEMPTS &&
    JSON.stringify(body) === JSON.stringify(["CANVAS"]);
  if (!recovered || failures.length > 0) {
    throw new Error(JSON.stringify({ recovered, injectedWorkers, probe, body, failures }, null, 2));
  }
  console.log(
    JSON.stringify({
      ok: true,
      recovered: true,
      failedAcquisitions: probe.failures,
      acquisitionCalls: probe.calls,
      canvasOnly: true,
      consoleErrors: 0,
    }),
  );
} catch (error) {
  console.error(JSON.stringify({ ok: false, error: String(error), failures }, null, 2));
  process.exitCode = 1;
} finally {
  await browser?.close();
  await server.close();
}
