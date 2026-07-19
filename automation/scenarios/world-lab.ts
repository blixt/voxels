import { BrowserCapability } from "../lib/browser.ts";
import { type EngineClient, snapshotValue } from "../lib/engine.ts";
import { defineScenario, type ScenarioContext } from "../lib/scenario.ts";
import { startWorldStack } from "../lib/world.ts";

const VIEWPORT = { width: 1280, height: 720 };

function near(value: number, target: number, tolerance = 0.002): boolean {
  return Math.abs(value - target) <= tolerance;
}

async function waitForSettledWorld(engine: EngineClient): Promise<readonly number[]> {
  return engine.waitForSnapshot(
    (snapshot) =>
      snapshotValue(snapshot, "allLodsReady") === 1 &&
      snapshotValue(snapshot, "pendingJobs") === 0 &&
      snapshotValue(snapshot, "residentChunks") > 0,
    { description: "World Lab fixture did not settle", timeoutMs: 60_000 },
  );
}

async function runWorldLab(context: ScenarioContext, arguments_: readonly string[]) {
  if (arguments_.length > 0) {
    throw new Error(`world-lab takes no arguments; received ${arguments_.join(" ")}`);
  }
  const world = await startWorldStack(context, {
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
  const viewport = await browser.open({
    url: world.url,
    label: "world-lab",
    viewport: VIEWPORT,
    ...world.clientRoute,
  });
  const { page } = viewport;
  const settled = await waitForSettledWorld(viewport.engine);
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
  const overridden = await viewport.engine.waitForSnapshot(
    (snapshot) =>
      near(snapshotValue(snapshot, "dayFraction"), 0.72) &&
      near(snapshotValue(snapshot, "localSolarDayFraction"), 0.72) &&
      snapshotValue(snapshot, "sunDirectionX") < -0.95 &&
      near(snapshotValue(snapshot, "twinklePhase"), expectedTwinklePhase, 0.000_01) &&
      near(snapshotValue(snapshot, "weatherFraction"), 0.68),
    { description: "time/weather override did not reach the renderer" },
  );

  await page.mouse.click(1_006, 352);
  const spectating = await viewport.engine.waitForSnapshot(
    (snapshot) => snapshotValue(snapshot, "spectatorActive") === 1,
    { description: "server-authorized spectator mode did not activate" },
  );
  await page.keyboard.press("F3");
  await page.waitForTimeout(100);
  const bodyPosition = [
    snapshotValue(spectating, "cameraX"),
    snapshotValue(spectating, "cameraY"),
    snapshotValue(spectating, "cameraZ"),
  ] as const;
  await page.keyboard.down("Space");
  await page.waitForTimeout(350);
  await page.keyboard.up("Space");
  const ascended = await viewport.engine.waitForSnapshot(
    (snapshot) => snapshotValue(snapshot, "cameraY") > bodyPosition[1] + 0.5,
    { description: "spectator ascent did not move the camera" },
  );

  await page.keyboard.press("F3");
  await page.waitForTimeout(100);
  await page.mouse.click(1_006, 352);
  await viewport.engine.waitForSnapshot(
    (snapshot) =>
      snapshotValue(snapshot, "spectatorActive") === 0 &&
      Math.hypot(
        snapshotValue(snapshot, "cameraX") - bodyPosition[0],
        snapshotValue(snapshot, "cameraY") - bodyPosition[1],
        snapshotValue(snapshot, "cameraZ") - bodyPosition[2],
      ) < 0.001,
    { description: "spectator mode did not restore the saved body position" },
  );
  await page.mouse.click(813.5, 205);
  await page.mouse.click(813.5, 268);
  const restored = await viewport.engine.waitForSnapshot(
    (snapshot) =>
      near(snapshotValue(snapshot, "dayFraction"), 0.5) &&
      near(snapshotValue(snapshot, "weatherFraction"), 0.08),
    { description: "server environment did not resume after selecting LIVE" },
  );
  browser.assertHealthy();

  return {
    summary: "World Lab controls and synchronized environment passed.",
    metrics: {
      residentChunks: snapshotValue(settled, "residentChunks"),
      ascentMetres: snapshotValue(ascended, "cameraY") - bodyPosition[1],
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
        spectatorActive: snapshotValue(restored, "spectatorActive"),
      },
    },
  };
}

export default defineScenario({
  id: "world-lab",
  kind: "validation",
  summary: "Exercises Rust World Lab controls and synchronized environment state.",
  uses: { world: true, browser: true, viewport: "browser", screenshots: true, rust: true },
  timeoutMs: 360_000,
  run: runWorldLab,
});
