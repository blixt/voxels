import { readFile } from "node:fs/promises";
import { resolve } from "node:path";
import { BrowserCapability } from "../lib/browser.ts";
import { ScenarioArguments } from "../lib/arguments.ts";
import { snapshotValue } from "../lib/engine.ts";
import { compareRenderedImages } from "../lib/image.ts";
import { readPlayerScreenshot, takePlayerScreenshot } from "../lib/player-screenshot.ts";
import { defineScenario, type ScenarioContext } from "../lib/scenario.ts";
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
    readonly animationTimeSeconds: number;
    readonly targetVolume: {
      readonly minimumVoxel: readonly [number, number, number];
      readonly maximumVoxel: readonly [number, number, number];
      readonly anchorVoxel: readonly [number, number, number];
      readonly shapeId: number;
    } | null;
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
    !Number.isFinite(value.render?.animationTimeSeconds) ||
    value.render.animationTimeSeconds < 0
  ) {
    throw new Error("capture renderer animation time is invalid");
  }
  const targetVolume = value.render?.targetVolume;
  const validVoxelCoordinate = (coordinate: unknown): boolean =>
    Array.isArray(coordinate) && coordinate.length === 3 && coordinate.every(Number.isSafeInteger);
  if (
    !Object.hasOwn(value.render, "targetVolume") ||
    (targetVolume !== null &&
      (!validVoxelCoordinate(targetVolume?.minimumVoxel) ||
        !validVoxelCoordinate(targetVolume?.maximumVoxel) ||
        !validVoxelCoordinate(targetVolume?.anchorVoxel) ||
        ![0, 1].includes(targetVolume?.shapeId)))
  ) {
    throw new Error("capture renderer target volume is invalid");
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
  const url = arguments_.string("url");
  arguments_.assertEmpty();
  if (input === undefined) {
    throw new Error("replay-screenshot requires --input=/absolute/path/to/capture.png");
  }
  if (url === undefined) {
    throw new Error(
      "replay-screenshot requires --url=http://... for the same authoritative world; a PNG intentionally does not embed a mutable multiplayer database or credentials",
    );
  }
  const parsedUrl = new URL(url);
  if (!["http:", "https:"].includes(parsedUrl.protocol)) {
    throw new Error("replay-screenshot --url must use http or https");
  }
  const capture = await readFile(resolve(input));
  const source = readPlayerScreenshot(capture, resolve(input));
  const metadataText = readPngText(capture, "voxels.reproduction");
  if (metadataText === undefined) {
    throw new Error("capture has no voxels.reproduction PNG metadata");
  }
  const metadata = parseMetadata(metadataText);
  const browser = await BrowserCapability.start(context);
  const viewport = await browser.open({
    url: parsedUrl.href,
    label: "screenshot-reproduction",
    viewport: {
      width: metadata.image.cssWidth,
      height: metadata.image.cssHeight,
    },
    deviceScaleFactor: metadata.image.devicePixelRatio,
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
  const reproduced = await takePlayerScreenshot(viewport.page, { timeoutMs: 45_000 });
  const expectedPages = metadata.presentation.selectedCut.cut?.selectedPages;
  const actualPages = reproduced.metadata.presentation.selectedCut.cut?.selectedPages;
  if (JSON.stringify(actualPages) !== JSON.stringify(expectedPages)) {
    throw new Error(
      "reproduced terrain cut fingerprint collided with a different selected page set",
    );
  }
  const imageComparison = await compareRenderedImages(viewport.page, source.png, reproduced.png, {
    region: { x0: 0.03, x1: 0.97, y0: 0.06, y1: 0.9 },
    footprintPixels: 4,
    diagnosticGeometry: true,
  });
  const ownershipAttachmentMatches =
    source.ownership !== null &&
    reproduced.ownership !== null &&
    source.ownership.width === reproduced.ownership.width &&
    source.ownership.height === reproduced.ownership.height &&
    Buffer.from(source.ownership.pixels).equals(Buffer.from(reproduced.ownership.pixels));
  await context.artifacts.writeJson("Fresh-browser replay comparison", "replay-comparison.json", {
    ...imageComparison,
    ownershipAttachmentMatches,
  });
  if (
    !ownershipAttachmentMatches ||
    imageComparison.ssim < 0.99 ||
    imageComparison.meanAbsoluteLinearRgbDelta > 0.005 ||
    imageComparison.meanAbsoluteLinearLumaDelta > 0.005 ||
    imageComparison.relativeMeanLinearLumaDelta > 0.02 ||
    imageComparison.diagnosticGeometry === null ||
    imageComparison.diagnosticGeometry.occupancyJaccard < 0.9999 ||
    imageComparison.diagnosticGeometry.largestDisagreementComponentPixels > 16
  ) {
    throw new Error(
      `fresh-browser replay changed terrain ownership, geometry, or appearance: ${JSON.stringify({ ...imageComparison, ownershipAttachmentMatches })}`,
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
      "Restored and froze the exact v3 camera, projection, environment, viewport, and terrain cut against the capture's authoritative world.",
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
  summary: "Replay embedded screenshot metadata against the same authoritative mutable world URL.",
  uses: { browser: true, viewport: "browser", screenshots: true },
  timeoutMs: 120_000,
  run,
});
