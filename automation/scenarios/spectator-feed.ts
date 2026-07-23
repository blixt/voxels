import { existsSync } from "node:fs";
import { mkdir } from "node:fs/promises";
import path from "node:path";
import { ScenarioArguments } from "../lib/arguments.ts";
import { BrowserCapability } from "../lib/browser.ts";
import { snapshotValue } from "../lib/engine.ts";
import { analyzeDiagnosticSky } from "../lib/image.ts";
import { defineScenario, type ScenarioContext } from "../lib/scenario.ts";
import { startWorldStack, type WorldClientRoute } from "../lib/world.ts";
import { namedPlayerUrl } from "../../web/local-player.ts";

const VIEWPORT = { width: 1_920, height: 1_080 };
const CAMERA_SETTLE_TIMEOUT_MS = 60_000;

type FeedMotion = "stationary" | "forward" | "orbit" | "rise";

interface FeedTarget {
  readonly url: string;
  readonly clientRoute?: WorldClientRoute;
  readonly external: boolean;
}

function feedUrl(source: string): URL {
  let url: URL;
  try {
    url = new URL(source);
  } catch {
    throw new Error(`--url is not a valid absolute URL: ${source}`);
  }
  if (url.protocol !== "http:" && url.protocol !== "https:") {
    throw new Error("--url must use HTTP or HTTPS");
  }
  return url;
}

async function feedTarget(
  context: ScenarioContext,
  source: string | undefined,
): Promise<FeedTarget> {
  if (source !== undefined) {
    return { url: feedUrl(source).href, external: true };
  }
  const world = await startWorldStack(context, {
    fixture: { source: "procedural-v16" },
    web: { buildProfile: "wasm-dev" },
  });
  return {
    url: world.url,
    clientRoute: world.clientRoute,
    external: false,
  };
}

function position(snapshot: readonly number[]): readonly [number, number, number] {
  return [
    snapshotValue(snapshot, "cameraX"),
    snapshotValue(snapshot, "cameraY"),
    snapshotValue(snapshot, "cameraZ"),
  ];
}

function distance(
  first: readonly [number, number, number],
  second: readonly [number, number, number],
): number {
  return Math.hypot(first[0] - second[0], first[1] - second[1], first[2] - second[2]);
}

async function runMotion(
  context: ScenarioContext,
  viewport: Awaited<ReturnType<BrowserCapability["open"]>>,
  motion: FeedMotion,
  durationSeconds: number,
): Promise<void> {
  const { page } = viewport;
  const key =
    motion === "forward" || motion === "orbit" ? "KeyW" : motion === "rise" ? "Space" : null;
  if (key !== null) await page.keyboard.down(key);
  try {
    const deadline = performance.now() + durationSeconds * 1_000;
    do {
      if (motion === "orbit") await viewport.engine.look(9, 0);
      await context.wait(Math.min(50, Math.max(deadline - performance.now(), 0)));
    } while (performance.now() < deadline);
  } finally {
    if (key !== null) await page.keyboard.up(key);
  }
}

async function runSpectatorFeed(context: ScenarioContext, arguments_: readonly string[]) {
  const options = new ScenarioArguments(arguments_);
  const source = options.string("url");
  const player = options.string("player", "spectator-feed")!;
  const durationSeconds = options.number("duration", {
    fallback: 8,
    minimum: 0.1,
    maximum: 3_600,
  })!;
  const motion = options.choice(
    "motion",
    ["stationary", "forward", "orbit", "rise"] as const,
    "orbit",
  );
  const look = options.pair("look", {
    minimum: -Math.PI,
    maximum: Math.PI,
  });
  const noVideo = options.flag("no-video");
  const validateGroundCoverage = options.flag("ground-coverage");
  const sessionStateOption = options.string("session-state");
  options.assertEmpty();
  if (look !== undefined && Math.abs(look[1]) > 1.5) {
    throw new Error("--look pitch must be within -1.5..=1.5 radians");
  }

  const target = await feedTarget(context, source);
  const url = namedPlayerUrl(player, target.url).href;
  const sessionStatePath =
    sessionStateOption === undefined ? undefined : path.resolve(sessionStateOption);
  const browser = await BrowserCapability.start(context);
  const viewport = await browser.open({
    url,
    label: "spectator-feed",
    viewport: VIEWPORT,
    recordVideo: !noVideo,
    videoFilename: "spectator-feed.webm",
    ...(sessionStatePath !== undefined && existsSync(sessionStatePath)
      ? { storageState: sessionStatePath }
      : {}),
    ...target.clientRoute,
  });
  const settled = await viewport.engine.waitForSnapshot(
    (snapshot) =>
      snapshotValue(snapshot, "allLodsReady") === 1 &&
      snapshotValue(snapshot, "residentChunks") > 0,
    {
      timeoutMs: CAMERA_SETTLE_TIMEOUT_MS,
      description: "spectator feed world did not become renderable",
    },
  );
  const bodyPosition = position(settled);
  const spectating = await viewport.engine.setSpectator(true);
  if (
    (await viewport.engine.submitPlace(0, 0, 0, 1, "cube")) ||
    (await viewport.engine.submitDig(0, 0, 0, "sphere"))
  ) {
    throw new Error("spectator feed retained a world-edit path");
  }
  if (look !== undefined) await viewport.engine.setCameraLook(look[0], look[1]);

  if (validateGroundCoverage) {
    const pitch = snapshotValue(await viewport.engine.snapshot(), "pitch");
    if (pitch > -1.45) {
      throw new Error("--ground-coverage requires a downward --look pitch at or below -1.45");
    }
    await viewport.engine.setDiagnosticSky([255, 0, 255]);
  }

  await viewport.screenshot("Spectator feed start", {
    filename: "spectator-feed-start.png",
  });
  await runMotion(context, viewport, motion, durationSeconds);
  const end = await viewport.engine.waitForSnapshot(
    (snapshot) => snapshotValue(snapshot, "spectatorActive") === 1,
    { description: "spectator role was lost during feed capture" },
  );
  if (validateGroundCoverage) {
    await viewport.engine.waitForSnapshot(
      (snapshot) =>
        snapshotValue(snapshot, "allLodsReady") === 1 &&
        snapshotValue(snapshot, "pendingJobs") === 0 &&
        snapshotValue(snapshot, "surfaceInFlight") === 0,
      {
        timeoutMs: CAMERA_SETTLE_TIMEOUT_MS,
        description: "aerial ground coverage did not settle after spectator motion",
      },
    );
    // Let the renderer's intentionally short complete-cut transition expire. Persistent seam
    // validation and moving-transition validation are separate assertions.
    await viewport.engine.wait(500);
  }
  await viewport.screenshot("Spectator feed end", {
    filename: "spectator-feed-end.png",
  });
  const groundCoverageScreenshot = validateGroundCoverage
    ? await viewport.page.screenshot()
    : undefined;
  const groundCoverage =
    groundCoverageScreenshot === undefined
      ? undefined
      : await analyzeDiagnosticSky(viewport.page, groundCoverageScreenshot, {
          x0: 0.02,
          x1: 0.98,
          y0: 0.08,
          y1: 0.98,
        });
  if (groundCoverageScreenshot !== undefined) {
    await context.artifacts.write(
      "Aerial diagnostic ground coverage",
      "spectator-ground-coverage.png",
      groundCoverageScreenshot,
      "image/png",
    );
  }
  await viewport.engine.setDiagnosticSky(null);

  const restored = await viewport.engine.setSpectator(false);
  const restoredPosition = position(restored);
  if (distance(bodyPosition, restoredPosition) > 0.001) {
    throw new Error(
      `leaving spectator restored ${JSON.stringify(restoredPosition)} instead of ${JSON.stringify(bodyPosition)}`,
    );
  }
  if (sessionStatePath !== undefined) {
    await mkdir(path.dirname(sessionStatePath), { recursive: true });
    await viewport.context.storageState({ path: sessionStatePath });
  }
  browser.assertHealthy();

  if (groundCoverage !== undefined && groundCoverage.diagnosticSkyPixels > 0) {
    throw new Error(
      `a downward aerial view exposed ${groundCoverage.diagnosticSkyPixels} diagnostic-sky pixels`,
    );
  }

  const startPosition = position(spectating);
  const endPosition = position(end);
  return {
    summary: target.external
      ? "Captured a read-only spectator feed from the running world."
      : "Captured a read-only spectator feed from an isolated world.",
    metrics: {
      durationSeconds,
      cameraDistanceMetres: distance(startPosition, endPosition),
      cameraAltitudeDeltaMetres: endPosition[1] - startPosition[1],
      residentChunks: snapshotValue(end, "residentChunks"),
      remoteAvatars: snapshotValue(end, "remoteAvatars"),
    },
    details: {
      browser: browser.version,
      externalWorld: target.external,
      player: await viewport.engine.playerSession(),
      motion,
      bodyPosition,
      startPosition,
      endPosition,
      restoredPosition,
      recordedVideo: !noVideo,
      groundCoverage: groundCoverage ?? null,
      sessionStatePath: sessionStatePath ?? null,
    },
  };
}

export default defineScenario({
  id: "spectator-feed",
  kind: "capture",
  summary: "Attaches a read-only flying camera to a world for regional screenshots or video.",
  uses: {
    world: true,
    browser: true,
    viewport: "browser",
    screenshots: true,
    video: true,
    rust: true,
  },
  timeoutMs: 3_900_000,
  run: runSpectatorFeed,
});
