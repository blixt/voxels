import { readFile } from "node:fs/promises";
import { resolve } from "node:path";
import { BrowserCapability } from "../lib/browser.ts";
import { ScenarioArguments } from "../lib/arguments.ts";
import { snapshotValue } from "../lib/engine.ts";
import { takePlayerScreenshot } from "../lib/player-screenshot.ts";
import { defineScenario, type ScenarioContext } from "../lib/scenario.ts";
import { startWorldStack, type WorldSource } from "../lib/world.ts";
import { readPngText } from "../../web/png-metadata.ts";
import type { WasmBuildProfile } from "../../scripts/build-wasm.ts";

interface ReproductionMetadata {
  readonly schema: string;
  readonly runtime: {
    readonly buildProfile: WasmBuildProfile;
  };
  readonly image: {
    readonly pixelWidth: number;
    readonly pixelHeight: number;
    readonly cssWidth: number;
    readonly cssHeight: number;
    readonly devicePixelRatio: number;
  };
  readonly camera: {
    readonly eyeMetres: readonly [number, number, number];
    readonly yawRadians: number;
    readonly pitchRadians: number;
  };
  readonly world: {
    readonly sourceKind: number;
  };
  readonly environment: {
    readonly worldDays: number;
    readonly dayFraction: number;
    readonly planetCircumferenceMetres: number;
    readonly axialTiltRadians: number;
    readonly moonOrbitInclinationRadians: number;
    readonly celestialSeed: string;
    readonly celestialRevision: string;
    readonly weatherFraction: number;
    readonly cloudVelocityMetresPerSecond: readonly [number, number];
    readonly cloudCoverage: number;
    readonly cloudBaseMetres: number;
    readonly cloudTopMetres: number;
  };
  readonly render: {
    readonly diagnosticSkyColor: readonly [number, number, number] | null;
    readonly features: {
      readonly shadows: boolean;
      readonly screenSpaceAmbientOcclusion: boolean;
    };
  };
  readonly presentation: {
    readonly selectedCutFingerprint: string;
    readonly selectedCut: {
      readonly kind: "virtualTerrain";
      readonly cut: {
        readonly selectedPages: readonly {
          readonly level: number;
          readonly coord: readonly [number, number, number];
        }[];
      } | null;
    };
  };
}

function finite(value: unknown, field: string): number {
  if (typeof value !== "number" || !Number.isFinite(value)) {
    throw new Error(`capture ${field} is not finite`);
  }
  return value;
}

function safeU64(value: string, field: string): number {
  let parsed: bigint;
  try {
    parsed = BigInt(value);
  } catch {
    throw new Error(`capture ${field} is not an unsigned integer`);
  }
  if (parsed < 0n || parsed > BigInt(Number.MAX_SAFE_INTEGER)) {
    throw new Error(`capture ${field} cannot be represented by the current fixture CLI`);
  }
  return Number(parsed);
}

function parseMetadata(text: string): ReproductionMetadata {
  const value = JSON.parse(text) as ReproductionMetadata;
  if (value.schema !== "voxels.reproduction.v3") {
    throw new Error(`unsupported screenshot reproduction schema ${String(value.schema)}`);
  }
  if (!["debug", "wasm-dev", "release"].includes(value.runtime?.buildProfile)) {
    throw new Error(
      `capture build profile ${String(value.runtime?.buildProfile)} is not reproducible`,
    );
  }
  const cssWidth = finite(value.image?.cssWidth, "CSS width");
  const cssHeight = finite(value.image?.cssHeight, "CSS height");
  const dpr = finite(value.image?.devicePixelRatio, "device pixel ratio");
  if (
    !Number.isSafeInteger(cssWidth) ||
    !Number.isSafeInteger(cssHeight) ||
    cssWidth <= 0 ||
    cssHeight <= 0 ||
    dpr <= 0 ||
    Math.round(cssWidth * dpr) !== value.image.pixelWidth ||
    Math.round(cssHeight * dpr) !== value.image.pixelHeight
  ) {
    throw new Error("capture viewport cannot be represented exactly by Playwright");
  }
  if (
    value.camera?.eyeMetres?.length !== 3 ||
    !value.camera.eyeMetres.every(Number.isFinite) ||
    !Number.isFinite(value.camera.yawRadians) ||
    !Number.isFinite(value.camera.pitchRadians)
  ) {
    throw new Error("capture omitted the exact camera pose");
  }
  const diagnosticSkyColor = value.render?.diagnosticSkyColor;
  if (
    diagnosticSkyColor !== null &&
    (diagnosticSkyColor?.length !== 3 ||
      diagnosticSkyColor.some((channel) => !Number.isFinite(channel) || channel < 0 || channel > 1))
  ) {
    throw new Error("capture diagnostic sky color is invalid");
  }
  if (
    !/^[0-9a-f]{16}$/u.test(value.presentation?.selectedCutFingerprint) ||
    value.presentation.selectedCut?.kind !== "virtualTerrain" ||
    !Array.isArray(value.presentation.selectedCut.cut?.selectedPages)
  ) {
    throw new Error("capture omitted the exact virtual terrain cut");
  }
  return value;
}

async function run(context: ScenarioContext, raw: readonly string[]) {
  const arguments_ = new ScenarioArguments(raw);
  const input = arguments_.string("input");
  const reuseBuild = arguments_.flag("reuse-build");
  arguments_.assertEmpty();
  if (input === undefined) {
    throw new Error("replay-screenshot requires --input=/absolute/path/to/capture.png");
  }
  const capture = await readFile(resolve(input));
  const metadataText = readPngText(capture, "voxels.reproduction");
  if (metadataText === undefined) {
    throw new Error("capture has no voxels.reproduction PNG metadata");
  }
  const metadata = parseMetadata(metadataText);
  const source: WorldSource =
    metadata.world.sourceKind === 1
      ? "procedural-v16"
      : metadata.world.sourceKind === 2
        ? "terrain-diffusion-30m"
        : (() => {
            throw new Error(`capture has unknown world source kind ${metadata.world.sourceKind}`);
          })();
  const worldDayNumber = Math.floor(metadata.environment.worldDays);
  const world = await startWorldStack(context, {
    fixture: {
      prefix: "voxels-reproduction-",
      source,
      dayLengthSeconds: 0,
      worldDayNumberAtUnixEpoch: worldDayNumber,
      dayFractionAtUnixEpoch: metadata.environment.dayFraction,
      planetCircumferenceMetres: metadata.environment.planetCircumferenceMetres,
      axialTiltDegrees: (metadata.environment.axialTiltRadians * 180) / Math.PI,
      moonOrbitInclinationDegrees:
        (metadata.environment.moonOrbitInclinationRadians * 180) / Math.PI,
      celestialSeed: safeU64(metadata.environment.celestialSeed, "celestial seed"),
      celestialRevision: safeU64(metadata.environment.celestialRevision, "celestial revision"),
      weatherCycleSeconds: 0,
      weatherFractionAtUnixEpoch: metadata.environment.weatherFraction,
      cloudVelocityMetresPerSecond: metadata.environment.cloudVelocityMetresPerSecond,
      cloudCoverage: metadata.environment.cloudCoverage,
      cloudBaseMetres: metadata.environment.cloudBaseMetres,
      cloudTopMetres: metadata.environment.cloudTopMetres,
      cascadedShadows: metadata.render.features.shadows,
      screenSpaceAmbientOcclusion: metadata.render.features.screenSpaceAmbientOcclusion,
      ...(metadata.render.diagnosticSkyColor === null
        ? {}
        : {
            diagnosticSkyRgb: metadata.render.diagnosticSkyColor.map((channel) =>
              Math.round(channel * 255),
            ) as [number, number, number],
          }),
    },
    service: { metal: source === "terrain-diffusion-30m" },
    web: {
      build: !reuseBuild,
      buildProfile: metadata.runtime.buildProfile,
    },
  });
  const browser = await BrowserCapability.start(context);
  const viewport = await browser.open({
    url: world.url,
    label: "screenshot-reproduction",
    viewport: {
      width: metadata.image.cssWidth,
      height: metadata.image.cssHeight,
    },
    deviceScaleFactor: metadata.image.devicePixelRatio,
    ...world.clientRoute,
  });
  await viewport.engine.applyReproduction(metadataText);
  const expectedCutLow48 =
    BigInt(`0x${metadata.presentation.selectedCutFingerprint}`) & 0xffffffffffffn;
  const expectedCutLow24 = Number(expectedCutLow48 & 0xffffffn);
  const expectedCutHigh24 = Number((expectedCutLow48 >> 24n) & 0xffffffn);
  const applied = await viewport.engine.waitForSnapshot(
    (snapshot) =>
      snapshotValue(snapshot, "residentChunks") > 0 &&
      snapshotValue(snapshot, "frameSequence") > 0 &&
      snapshotValue(snapshot, "clientViewGoalKind") === 0 &&
      snapshotValue(snapshot, "virtualTerrainCutFingerprintLow24") === expectedCutLow24 &&
      snapshotValue(snapshot, "virtualTerrainCutFingerprintHigh24") === expectedCutHigh24,
    {
      timeoutMs: 60_000,
      description: "reproduction did not commit the captured terrain cut",
    },
  );
  await viewport.engine.waitForFrameAfter(snapshotValue(applied, "frameSequence"));
  const reproduced = await takePlayerScreenshot(viewport.page);
  const expectedPages = metadata.presentation.selectedCut.cut?.selectedPages;
  const actualPages = reproduced.metadata.presentation.selectedCut.cut?.selectedPages;
  if (JSON.stringify(actualPages) !== JSON.stringify(expectedPages)) {
    throw new Error(
      "reproduced terrain cut fingerprint collided with a different selected page set",
    );
  }
  const artifact = await context.artifacts.write(
    "Reproduced screenshot",
    "reproduced.png",
    reproduced.png,
    "image/png",
  );
  browser.assertHealthy();
  return {
    summary:
      "Restored and froze the exact v3 camera, projection, world, environment, and viewport.",
    details: {
      input: resolve(input),
      artifact,
      camera: {
        eyeMetres: metadata.camera.eyeMetres,
        yawRadians: metadata.camera.yawRadians,
        pitchRadians: metadata.camera.pitchRadians,
      },
    },
  };
}

export default defineScenario({
  id: "replay-screenshot",
  kind: "capture",
  summary: "Replay an embedded voxels.reproduction.v3 screenshot state exactly.",
  uses: { world: true, browser: true, viewport: "browser", screenshots: true },
  timeoutMs: 180_000,
  run,
});
