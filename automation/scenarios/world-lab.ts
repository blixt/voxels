import type { Page } from "playwright";
import { BrowserCapability } from "../lib/browser.ts";
import { snapshotValue } from "../lib/engine.ts";
import { defineScenario, type ScenarioContext } from "../lib/scenario.ts";
import { startWorldPreview } from "../lib/world.ts";

const VIEWPORT = { width: 1280, height: 720 };

function near(value: number, target: number, tolerance = 0.002): boolean {
  return Math.abs(value - target) <= tolerance;
}

async function waitForSnapshot(
  page: Page,
  predicate: (snapshot: readonly number[]) => boolean,
  description: string,
  timeoutMs = 5_000,
): Promise<readonly number[]> {
  const deadline = Date.now() + timeoutMs;
  let latest: readonly number[] = [];
  while (Date.now() < deadline) {
    latest = await page.evaluate(() => globalThis.__VOXELS__!.snapshot());
    if (predicate(latest)) return latest;
    await page.waitForTimeout(25);
  }
  throw new Error(`${description}: ${JSON.stringify(latest)}`);
}

async function waitForSettledWorld(page: Page): Promise<readonly number[]> {
  return waitForSnapshot(
    page,
    (snapshot) =>
      snapshotValue(snapshot, "allLodsReady") === 1 &&
      snapshotValue(snapshot, "pendingJobs") === 0 &&
      snapshotValue(snapshot, "residentChunks") > 0,
    "World Lab fixture did not settle",
    60_000,
  );
}

async function runWorldLab(context: ScenarioContext, arguments_: readonly string[]) {
  if (arguments_.length > 0) {
    throw new Error(`world-lab takes no arguments; received ${arguments_.join(" ")}`);
  }
  const world = await startWorldPreview(context, {
    fixture: {
      prefix: "voxels-world-lab-",
      source: "procedural-v16",
      dayLengthSeconds: 0,
      worldDayNumberAtUnixEpoch: 0,
      dayFractionAtUnixEpoch: 0.5,
      moonOrbitPhaseAtWorldEpoch: 0.5,
      weatherCycleSeconds: 0,
      weatherFractionAtUnixEpoch: 0.08,
    },
  });
  const browser = await BrowserCapability.start(context);
  const viewport = await browser.open({ url: world.url, label: "world-lab", viewport: VIEWPORT });
  const { page } = viewport;
  const settled = await waitForSettledWorld(page);
  const expectedYearFraction = 0.5 / world.fixture.daysPerYear;
  const expectedMoonOrbitFraction =
    (0.5 / world.fixture.moonSiderealOrbitDays + world.fixture.moonOrbitPhaseAtWorldEpoch) % 1;
  const expectedTwinklePhase = (0.5 * 37) % 1;
  if (
    !near(snapshotValue(settled, "localSolarDayFraction"), 0.5) ||
    !near(snapshotValue(settled, "yearFraction"), expectedYearFraction, 0.000_01) ||
    !near(snapshotValue(settled, "moonOrbitFraction"), expectedMoonOrbitFraction, 0.000_01) ||
    !near(snapshotValue(settled, "twinklePhase"), expectedTwinklePhase, 0.000_01) ||
    !near(snapshotValue(settled, "latitudeDegrees"), 0, 0.01) ||
    !near(snapshotValue(settled, "longitudeDegrees"), 0, 0.01) ||
    Math.abs(snapshotValue(settled, "localSiderealAngleRadians")) > 0.02 ||
    snapshotValue(settled, "moonIlluminatedFraction") < 0.99 ||
    snapshotValue(settled, "celestialRevision") !== 1 ||
    snapshotValue(settled, "sunDirectionY") < 0.999 ||
    snapshotValue(settled, "moonDirectionY") > -0.99
  ) {
    throw new Error(`synchronized celestial anchor is incorrect: ${JSON.stringify(settled)}`);
  }

  await page.keyboard.press("F3");
  await page.waitForTimeout(250);
  await viewport.screenshot("World Lab", { filename: "world-lab.png" });

  // Rust owns these hit regions; renderer unit tests cover their responsive layout separately.
  await page.mouse.click(1_044.5, 205); // GOLDEN
  await page.mouse.click(1_198.5, 268); // STORM
  const overridden = await waitForSnapshot(
    page,
    (snapshot) =>
      near(snapshotValue(snapshot, "dayFraction"), 0.72) &&
      near(snapshotValue(snapshot, "localSolarDayFraction"), 0.72) &&
      snapshotValue(snapshot, "sunDirectionX") < -0.95 &&
      near(snapshotValue(snapshot, "twinklePhase"), expectedTwinklePhase, 0.000_01) &&
      near(snapshotValue(snapshot, "weatherFraction"), 0.68),
    "time/weather override did not reach the renderer",
  );

  await page.mouse.click(1_006, 352);
  const flying = await waitForSnapshot(
    page,
    (snapshot) => snapshotValue(snapshot, "creativeFlightActive") === 1,
    "server-authorized creative flight did not activate",
  );
  await page.keyboard.press("F3");
  await page.waitForTimeout(100);
  const initialY = snapshotValue(flying, "cameraY");
  await page.keyboard.down("Space");
  await page.waitForTimeout(350);
  await page.keyboard.up("Space");
  const ascended = await waitForSnapshot(
    page,
    (snapshot) => snapshotValue(snapshot, "cameraY") > initialY + 0.5,
    "creative-flight ascent did not move the player",
  );

  await page.keyboard.press("F3");
  await page.waitForTimeout(100);
  await page.mouse.click(1_006, 352);
  await waitForSnapshot(
    page,
    (snapshot) => snapshotValue(snapshot, "creativeFlightActive") === 0,
    "creative flight did not return to walking",
  );
  await page.mouse.click(813.5, 205);
  await page.mouse.click(813.5, 268);
  const restored = await waitForSnapshot(
    page,
    (snapshot) =>
      near(snapshotValue(snapshot, "dayFraction"), 0.5) &&
      near(snapshotValue(snapshot, "weatherFraction"), 0.08),
    "server environment did not resume after selecting LIVE",
  );
  browser.assertHealthy();

  return {
    summary: "World Lab controls and synchronized environment passed.",
    metrics: {
      residentChunks: snapshotValue(settled, "residentChunks"),
      ascentMetres: snapshotValue(ascended, "cameraY") - initialY,
    },
    details: {
      browser: browser.version,
      settled: {
        dayFraction: snapshotValue(settled, "dayFraction"),
        localSolarDayFraction: snapshotValue(settled, "localSolarDayFraction"),
        yearFraction: snapshotValue(settled, "yearFraction"),
        moonOrbitFraction: snapshotValue(settled, "moonOrbitFraction"),
        moonIlluminatedFraction: snapshotValue(settled, "moonIlluminatedFraction"),
        latitudeDegrees: snapshotValue(settled, "latitudeDegrees"),
        longitudeDegrees: snapshotValue(settled, "longitudeDegrees"),
        sunDirection: [
          snapshotValue(settled, "sunDirectionX"),
          snapshotValue(settled, "sunDirectionY"),
          snapshotValue(settled, "sunDirectionZ"),
        ],
        moonDirection: [
          snapshotValue(settled, "moonDirectionX"),
          snapshotValue(settled, "moonDirectionY"),
          snapshotValue(settled, "moonDirectionZ"),
        ],
        weatherFraction: snapshotValue(settled, "weatherFraction"),
      },
      overridden: {
        dayFraction: snapshotValue(overridden, "dayFraction"),
        localSolarDayFraction: snapshotValue(overridden, "localSolarDayFraction"),
        weatherFraction: snapshotValue(overridden, "weatherFraction"),
      },
      restored: {
        dayFraction: snapshotValue(restored, "dayFraction"),
        weatherFraction: snapshotValue(restored, "weatherFraction"),
        creativeFlightActive: snapshotValue(restored, "creativeFlightActive"),
      },
    },
  };
}

export default defineScenario({
  id: "world-lab",
  kind: "validation",
  summary: "Exercises Rust World Lab controls and synchronized environment state.",
  uses: { world: true, viewport: "browser", screenshots: true, rust: true },
  timeoutMs: 360_000,
  run: runWorldLab,
});
