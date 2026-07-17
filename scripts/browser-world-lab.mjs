import { execFileSync } from "node:child_process";
import { mkdir, writeFile } from "node:fs/promises";
import path from "node:path";
import { chromium } from "playwright";
import {
  assertSnapshotSchema,
  chromeWebGpuLaunchOptions,
  isBrowserConsoleFailure,
  reserveEphemeralPort,
  SNAPSHOT,
} from "./browser-harness.mjs";
import { prepareBrowserWorldFixture, startBrowserWorldService } from "./browser-world-fixture.mjs";

const FAILURE =
  /panic|unreachable|runtimeerror|wgpu|webgpu|shader|sqlite|opfs|syncaccesshandle|nomodificationallowed|web lock request failed|no persistence leader|persistence .*failed/i;
const VIEWPORT = { width: 1280, height: 720 };
const OUTPUT_DIRECTORY = path.resolve(process.env.VOXELS_WORLD_LAB_OUTPUT ?? "target/world-lab");

async function waitForEngine(page) {
  await page.waitForFunction(() => typeof globalThis.__VOXELS__?.snapshot === "function", null, {
    timeout: 20_000,
  });
  const deadline = Date.now() + 60_000;
  let latest = [];
  while (Date.now() < deadline) {
    latest = assertSnapshotSchema(await page.evaluate(() => globalThis.__VOXELS__.snapshot()));
    if (
      latest[SNAPSHOT.allLodsReady] === 1 &&
      latest[SNAPSHOT.pendingJobs] === 0 &&
      latest[SNAPSHOT.residentChunks] > 0
    ) {
      return latest;
    }
    await page.waitForTimeout(50);
  }
  throw new Error(`World Lab fixture did not settle: ${JSON.stringify(latest)}`);
}

async function waitForSnapshot(page, predicate, description) {
  const deadline = Date.now() + 5_000;
  let latest = [];
  while (Date.now() < deadline) {
    latest = assertSnapshotSchema(await page.evaluate(() => globalThis.__VOXELS__.snapshot()));
    if (predicate(latest)) return latest;
    await page.waitForTimeout(25);
  }
  throw new Error(`${description}: ${JSON.stringify(latest)}`);
}

function near(value, target, tolerance = 0.002) {
  return Math.abs(value - target) <= tolerance;
}

await mkdir(OUTPUT_DIRECTORY, { recursive: true });
const errors = [];
const port = await reserveEphemeralPort();
let browser;
let server;
let fixture;
let worldService;

try {
  fixture = await prepareBrowserWorldFixture({
    browserPort: port,
    prefix: "voxels-world-lab-",
    source: "procedural-v16",
    dayLengthSeconds: 0,
    dayFractionAtUnixEpoch: 0.5,
    weatherCycleSeconds: 0,
    weatherFractionAtUnixEpoch: 0.08,
  });
  const { build, preview } = await import("vite-plus");
  await build({ logLevel: "warn" });
  worldService = await startBrowserWorldService(fixture);
  server = await preview({
    logLevel: "warn",
    preview: { host: "127.0.0.1", port, strictPort: true },
  });
  browser = await chromium.launch(chromeWebGpuLaunchOptions());
  const context = await browser.newContext({ viewport: VIEWPORT, deviceScaleFactor: 1 });
  const page = await context.newPage();
  page.on("pageerror", (error) => errors.push(`pageerror: ${error.message}`));
  page.on("console", (message) => {
    if (isBrowserConsoleFailure(message.type(), message.text(), FAILURE)) {
      errors.push(`${message.type()}: ${message.text()}`);
    }
  });
  await page.goto(`http://127.0.0.1:${port}`, { waitUntil: "domcontentloaded" });
  const settled = await waitForEngine(page);

  await page.keyboard.press("F3");
  await page.waitForTimeout(250);
  await page.screenshot({ path: path.join(OUTPUT_DIRECTORY, "world-lab.png") });

  // These points are the centers of regions produced by the 1280x720 wide layout. Layout and hit
  // regions share one Rust source of truth; unit tests separately cover all responsive viewports.
  await page.mouse.click(1_044.5, 205); // GOLDEN
  await page.mouse.click(1_198.5, 268); // STORM
  const overridden = await waitForSnapshot(
    page,
    (snapshot) =>
      near(snapshot[SNAPSHOT.dayFraction], 0.72) && near(snapshot[SNAPSHOT.weatherFraction], 0.68),
    "time/weather override did not reach the renderer",
  );

  await page.mouse.click(1_006, 352); // Creative flight card
  const flying = await waitForSnapshot(
    page,
    (snapshot) => snapshot[SNAPSHOT.creativeFlightActive] === 1,
    "server-authorized creative flight did not activate",
  );
  await page.keyboard.press("F3");
  await page.waitForTimeout(100);
  const initialY = flying[SNAPSHOT.cameraY];
  await page.keyboard.down("Space");
  await page.waitForTimeout(350);
  await page.keyboard.up("Space");
  const ascended = await waitForSnapshot(
    page,
    (snapshot) => snapshot[SNAPSHOT.cameraY] > initialY + 0.5,
    "creative-flight ascent did not move the player",
  );

  await page.keyboard.press("F3");
  await page.waitForTimeout(100);
  await page.mouse.click(1_006, 352);
  await waitForSnapshot(
    page,
    (snapshot) => snapshot[SNAPSHOT.creativeFlightActive] === 0,
    "creative flight did not return to walking",
  );
  await page.mouse.click(813.5, 205); // LIVE time
  await page.mouse.click(813.5, 268); // LIVE weather
  const restored = await waitForSnapshot(
    page,
    (snapshot) =>
      near(snapshot[SNAPSHOT.dayFraction], 0.5) && near(snapshot[SNAPSHOT.weatherFraction], 0.08),
    "server environment did not resume after selecting LIVE",
  );

  if (errors.length > 0) throw new Error(errors.join("\n"));
  const result = {
    ok: true,
    commit: execFileSync("git", ["rev-parse", "HEAD"], { encoding: "utf8" }).trim(),
    dirty: execFileSync("git", ["status", "--porcelain"], { encoding: "utf8" }).length > 0,
    browser: browser.version(),
    settled: {
      dayFraction: settled[SNAPSHOT.dayFraction],
      weatherFraction: settled[SNAPSHOT.weatherFraction],
      residentChunks: settled[SNAPSHOT.residentChunks],
    },
    overridden: {
      dayFraction: overridden[SNAPSHOT.dayFraction],
      weatherFraction: overridden[SNAPSHOT.weatherFraction],
    },
    creativeFlight: {
      active: flying[SNAPSHOT.creativeFlightActive],
      ascentMetres: ascended[SNAPSHOT.cameraY] - initialY,
    },
    restored: {
      dayFraction: restored[SNAPSHOT.dayFraction],
      weatherFraction: restored[SNAPSHOT.weatherFraction],
      creativeFlightActive: restored[SNAPSHOT.creativeFlightActive],
    },
  };
  await writeFile(
    path.join(OUTPUT_DIRECTORY, "report.json"),
    `${JSON.stringify(result, null, 2)}\n`,
  );
  console.log(JSON.stringify(result, null, 2));
} finally {
  await browser?.close();
  await server?.close();
  await worldService?.close();
  await fixture?.cleanup();
}
