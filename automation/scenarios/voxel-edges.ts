import { execFileSync } from "node:child_process";
import { readFile } from "node:fs/promises";
import type { Page } from "playwright";
import { ScenarioArguments } from "../lib/arguments.ts";
import { BrowserCapability, chromeWebGpuLaunchOptions } from "../lib/browser.ts";
import { type EngineClient, snapshotValue } from "../lib/engine.ts";
import { defineScenario, type ScenarioContext } from "../lib/scenario.ts";
import {
  readTerrainDiagnosticAttachment,
  type TerrainDiagnosticAttachment,
} from "../lib/terrain-diagnostic.ts";
import { startWorldStack, type WorldSource } from "../lib/world.ts";
import { readPngText } from "../../web/png-metadata.ts";

const FAILURE =
  /panic|unreachable|runtimeerror|wgpu|webgpu|shader|sqlite|opfs|syncaccesshandle|nomodificationallowed|web lock request failed|no persistence leader|persistence .*failed|server rejected edit/iu;
const VOXELS_PER_METRE = 10;
const CUBE_LOWER_VOXELS = 4;
const CUBE_UPPER_VOXELS = 5;
const EDGE_TOLERANCE_PIXELS = 1;

type Vector3 = readonly [number, number, number];
type VoxelVector3 = readonly [number, number, number];

interface Options {
  readonly source: WorldSource;
  readonly buildProfile: "debug" | "wasm-dev" | "release";
  readonly viewport: { readonly width: number; readonly height: number };
  readonly deviceScaleFactor: number;
  readonly coordinateSpace: "positive" | "negative";
  readonly randomizedPoses: number;
}

function parseOptions(arguments_: readonly string[]): Options {
  const reader = new ScenarioArguments(arguments_);
  const viewport = reader.pair("viewport", {
    fallback: [1280, 720],
    separator: "x",
    integer: true,
    minimum: 480,
  });
  if (viewport === undefined) throw new Error("viewport default is missing");
  const options: Options = {
    source: reader.choice(
      "source",
      ["procedural-v16", "terrain-diffusion-30m"] as const,
      "procedural-v16",
    ),
    buildProfile: reader.choice("build", ["debug", "wasm-dev", "release"] as const, "release"),
    viewport: { width: viewport[0], height: viewport[1] },
    deviceScaleFactor: reader.number("dpr", { fallback: 1, minimum: 0.5, maximum: 4 }) ?? 1,
    coordinateSpace: reader.choice("coordinates", ["positive", "negative"] as const, "positive"),
    randomizedPoses:
      reader.number("poses", { fallback: 16, minimum: 1, maximum: 128, integer: true }) ?? 16,
  };
  reader.assertEmpty();
  return options;
}

function cameraPosition(snapshot: readonly number[]): Vector3 {
  return [
    snapshotValue(snapshot, "cameraX"),
    snapshotValue(snapshot, "cameraY"),
    snapshotValue(snapshot, "cameraZ"),
  ];
}

async function waitForSettledWorld(
  engine: EngineClient,
  description: string,
): Promise<readonly number[]> {
  let stableSamples = 0;
  let previousFingerprint: string | undefined;
  return engine.waitForSnapshot(
    (snapshot) => {
      const fingerprint = `${snapshotValue(snapshot, "viewportFingerprintHigh24")}:${snapshotValue(snapshot, "viewportFingerprintLow24")}:${snapshotValue(snapshot, "quads")}`;
      const ready =
        snapshotValue(snapshot, "quads") > 0 &&
        snapshotValue(snapshot, "canonicalImmediateResident") ===
          snapshotValue(snapshot, "canonicalImmediateRequired") &&
        snapshotValue(snapshot, "surfaceQueued") === 0 &&
        snapshotValue(snapshot, "surfaceDirty") === 0 &&
        snapshotValue(snapshot, "surfaceInFlight") === 0 &&
        snapshotValue(snapshot, "pendingJobs") === 0 &&
        snapshotValue(snapshot, "lodIncompleteTransitionEdges") === 0;
      stableSamples = ready && fingerprint === previousFingerprint ? stableSamples + 1 : 0;
      previousFingerprint = fingerprint;
      return stableSamples >= 3;
    },
    { timeoutMs: 90_000, intervalMs: 25, description },
  );
}

async function waitForEdit(
  engine: EngineClient,
  editsBefore: number,
  description: string,
): Promise<readonly number[]> {
  return engine.waitForSnapshot(
    (snapshot) => {
      const required = snapshotValue(snapshot, "editCanonicalRequired");
      return (
        snapshotValue(snapshot, "edits") > editsBefore &&
        snapshotValue(snapshot, "editCanonicalRenderable") === required &&
        snapshotValue(snapshot, "editCanonicalOwned") === required &&
        snapshotValue(snapshot, "pendingJobs") === 0 &&
        snapshotValue(snapshot, "surfaceInFlight") === 0
      );
    },
    { timeoutMs: 30_000, intervalMs: 25, description },
  );
}

async function submitDigIfSolid(
  engine: EngineClient,
  target: VoxelVector3,
  description: string,
): Promise<boolean> {
  const before = await engine.snapshot();
  const editsBefore = snapshotValue(before, "edits");
  if (!(await engine.submitDig(target[0], target[1], target[2], "cube"))) {
    throw new Error(`${description} was backpressured`);
  }
  const deadline = performance.now() + 2_000;
  while (performance.now() < deadline) {
    const after = await engine.snapshot();
    if (snapshotValue(after, "edits") > editsBefore) {
      await waitForEdit(engine, editsBefore, description);
      return true;
    }
    if (
      snapshotValue(after, "pendingJobs") === 0 &&
      snapshotValue(after, "surfaceInFlight") === 0
    ) {
      await engine.wait(100);
      const confirmed = await engine.snapshot();
      if (snapshotValue(confirmed, "edits") === editsBefore) return false;
    }
    await engine.wait(25);
  }
  throw new Error(`${description} neither remained empty nor converged`);
}

async function place(
  engine: EngineClient,
  target: VoxelVector3,
  materialId: number,
  description: string,
): Promise<void> {
  const before = await engine.snapshot();
  const editsBefore = snapshotValue(before, "edits");
  if (!(await engine.submitPlace(target[0], target[1], target[2], materialId, "cube"))) {
    throw new Error(`${description} was backpressured`);
  }
  await waitForEdit(engine, editsBefore, description);
}

async function moveForward(page: Page, engine: EngineClient, metres: number) {
  const before = await engine.snapshot();
  await page.keyboard.down("KeyW");
  try {
    await engine.waitForSnapshot(
      (snapshot) =>
        Math.hypot(
          snapshotValue(snapshot, "cameraX") - snapshotValue(before, "cameraX"),
          snapshotValue(snapshot, "cameraZ") - snapshotValue(before, "cameraZ"),
        ) >= metres,
      { timeoutMs: 20_000, intervalMs: 25, description: "edge fixture walk did not finish" },
    );
  } finally {
    await page.keyboard.up("KeyW");
  }
  return engine.snapshot();
}

async function collectMaterial(
  engine: EngineClient,
  origin: readonly number[],
  groundY: number,
  requiredUnits: number,
): Promise<number> {
  const originX = Math.round(snapshotValue(origin, "cameraX") * VOXELS_PER_METRE);
  const originZ = Math.round(snapshotValue(origin, "cameraZ") * VOXELS_PER_METRE);
  let inventory = await engine.inventory();
  for (const forward of [-12, -24, -36, -48] as const) {
    for (const right of [-18, -6, 6, 18] as const) {
      await submitDigIfSolid(
        engine,
        [originX + right, groundY - 3, originZ + forward],
        `edge fixture inventory dig ${forward}/${right}`,
      );
      inventory = await engine.inventory();
      const material = Array.from({ length: inventory.length - 2 }, (_unused, index) => index + 1)
        .filter((id) => (inventory[id + 1] ?? 0) >= requiredUnits)
        .sort((left, right) => (inventory[right + 1] ?? 0) - (inventory[left + 1] ?? 0))[0];
      if (material !== undefined) return material;
    }
  }
  throw new Error(
    `edge fixture did not collect ${requiredUnits} units of one material: ${JSON.stringify(inventory)}`,
  );
}

function aimAt(engine: EngineClient, snapshot: readonly number[], target: Vector3) {
  const camera = cameraPosition(snapshot);
  const dx = target[0] - camera[0];
  const dy = target[1] - camera[1];
  const dz = target[2] - camera[2];
  return engine.setCameraLook(Math.atan2(dx, -dz), Math.atan2(dy, Math.hypot(dx, dz)), {
    timeoutMs: 10_000,
    intervalMs: 10,
    description: "edge fixture camera did not align",
  });
}

function cubeBounds(first: VoxelVector3, second: VoxelVector3) {
  return {
    minimum: [
      Math.min(first[0], second[0]) - CUBE_LOWER_VOXELS,
      Math.min(first[1], second[1]) - CUBE_LOWER_VOXELS,
      Math.min(first[2], second[2]) - CUBE_LOWER_VOXELS,
    ].map((value) => value / VOXELS_PER_METRE) as [number, number, number],
    maximum: [
      Math.max(first[0], second[0]) + CUBE_UPPER_VOXELS + 1,
      Math.max(first[1], second[1]) + CUBE_UPPER_VOXELS + 1,
      Math.max(first[2], second[2]) + CUBE_UPPER_VOXELS + 1,
    ].map((value) => value / VOXELS_PER_METRE) as [number, number, number],
  };
}

function projectBox(
  snapshot: readonly number[],
  bounds: ReturnType<typeof cubeBounds>,
  width: number,
  height: number,
) {
  const position = cameraPosition(snapshot);
  const yaw = snapshotValue(snapshot, "yaw");
  const pitch = snapshotValue(snapshot, "pitch");
  const sinYaw = Math.sin(yaw);
  const cosYaw = Math.cos(yaw);
  const sinPitch = Math.sin(pitch);
  const cosPitch = Math.cos(pitch);
  const forward = [sinYaw * cosPitch, sinPitch, -cosYaw * cosPitch] as const;
  const right = [cosYaw, 0, sinYaw] as const;
  const up = [-sinYaw * sinPitch, cosPitch, cosYaw * sinPitch] as const;
  const tangent = Math.tan((68 * Math.PI) / 360);
  const aspect = width / height;
  const points: Array<readonly [number, number]> = [];
  for (const x of [bounds.minimum[0], bounds.maximum[0]]) {
    for (const y of [bounds.minimum[1], bounds.maximum[1]]) {
      for (const z of [bounds.minimum[2], bounds.maximum[2]]) {
        const delta: Vector3 = [x - position[0], y - position[1], z - position[2]];
        const depth = delta[0] * forward[0] + delta[1] * forward[1] + delta[2] * forward[2];
        if (depth <= 0.05) throw new Error("edge fixture box crossed the camera near plane");
        const viewX = delta[0] * right[0] + delta[1] * right[1] + delta[2] * right[2];
        const viewY = delta[0] * up[0] + delta[1] * up[1] + delta[2] * up[2];
        points.push([
          ((viewX / (depth * tangent * aspect)) * 0.5 + 0.5) * width,
          (0.5 - (viewY / (depth * tangent)) * 0.5) * height,
        ]);
      }
    }
  }
  return points;
}

async function analyzeProjectedBox(
  page: Page,
  screenshot: Buffer | undefined,
  projectedCorners: readonly (readonly [number, number])[],
  occupancyMask?: { readonly width: number; readonly height: number; readonly bytes: Uint8Array },
) {
  return page.evaluate(
    async ({ base64, maskBase64, maskWidth, maskHeight, corners, tolerance }) => {
      let imageWidth: number;
      let imageHeight: number;
      let pixels: Uint8ClampedArray | undefined;
      let fullMask: Uint8Array | undefined;
      if (maskBase64 !== undefined) {
        const binary = atob(maskBase64);
        fullMask = Uint8Array.from(binary, (character) => character.charCodeAt(0));
        imageWidth = maskWidth;
        imageHeight = maskHeight;
        if (fullMask.length !== imageWidth * imageHeight) {
          throw new Error("edge oracle occupancy mask has the wrong dimensions");
        }
      } else {
        if (base64 === undefined) throw new Error("edge oracle has no rendered source");
        const response = await fetch(`data:image/png;base64,${base64}`);
        const bitmap = await createImageBitmap(await response.blob());
        const canvas = new OffscreenCanvas(bitmap.width, bitmap.height);
        const context = canvas.getContext("2d", { willReadFrequently: true });
        if (context === null) throw new Error("edge oracle canvas is unavailable");
        context.drawImage(bitmap, 0, 0);
        imageWidth = bitmap.width;
        imageHeight = bitmap.height;
        pixels = context.getImageData(0, 0, bitmap.width, bitmap.height).data;
      }
      const cross = (
        origin: readonly number[],
        left: readonly number[],
        right: readonly number[],
      ) =>
        (left[0]! - origin[0]!) * (right[1]! - origin[1]!) -
        (left[1]! - origin[1]!) * (right[0]! - origin[0]!);
      const sorted = [...corners].sort((left, right) =>
        left[0] === right[0] ? left[1]! - right[1]! : left[0]! - right[0]!,
      );
      const half = (source: readonly (readonly number[])[]) => {
        const output: (readonly number[])[] = [];
        for (const point of source) {
          while (output.length >= 2 && cross(output.at(-2)!, output.at(-1)!, point) <= 0) {
            output.pop();
          }
          output.push(point);
        }
        return output;
      };
      const lower = half(sorted);
      const upper = half([...sorted].reverse());
      const hull = [...lower.slice(0, -1), ...upper.slice(0, -1)];
      const x0 = Math.max(0, Math.floor(Math.min(...hull.map((point) => point[0]!))) - 8);
      const x1 = Math.min(imageWidth, Math.ceil(Math.max(...hull.map((point) => point[0]!))) + 8);
      const y0 = Math.max(0, Math.floor(Math.min(...hull.map((point) => point[1]!))) - 8);
      const y1 = Math.min(imageHeight, Math.ceil(Math.max(...hull.map((point) => point[1]!))) + 8);
      const inside = (x: number, y: number) => {
        for (let index = 0; index < hull.length; index += 1) {
          if (cross(hull[index]!, hull[(index + 1) % hull.length]!, [x, y]) < 0) return false;
        }
        return true;
      };
      const expected = new Uint8Array((x1 - x0) * (y1 - y0));
      const actual = new Uint8Array(expected.length);
      const width = x1 - x0;
      for (let y = y0; y < y1; y += 1) {
        for (let x = x0; x < x1; x += 1) {
          const local = x - x0 + (y - y0) * width;
          expected[local] = Number(inside(x + 0.5, y + 0.5));
          if (fullMask !== undefined) {
            actual[local] = fullMask[x + y * imageWidth] ?? 0;
          } else {
            const source = (x + y * imageWidth) * 4;
            const red = pixels?.[source] ?? 0;
            const green = pixels?.[source + 1] ?? 0;
            const blue = pixels?.[source + 2] ?? 0;
            actual[local] = Number(!(red >= 232 && green <= 48 && blue >= 232));
          }
        }
      }
      const near = (mask: Uint8Array, x: number, y: number) => {
        for (let dy = -tolerance; dy <= tolerance; dy += 1) {
          for (let dx = -tolerance; dx <= tolerance; dx += 1) {
            const candidateX = x + dx;
            const candidateY = y + dy;
            if (
              candidateX >= 0 &&
              candidateX < width &&
              candidateY >= 0 &&
              candidateY < y1 - y0 &&
              mask[candidateX + candidateY * width] !== 0
            ) {
              return true;
            }
          }
        }
        return false;
      };
      let expectedPixels = 0;
      let actualPixels = 0;
      let missingPixels = 0;
      let excessPixels = 0;
      let missingBeyondTolerancePixels = 0;
      let excessBeyondTolerancePixels = 0;
      for (let y = 0; y < y1 - y0; y += 1) {
        for (let x = 0; x < width; x += 1) {
          const index = x + y * width;
          expectedPixels += expected[index] ?? 0;
          actualPixels += actual[index] ?? 0;
          if (expected[index] !== 0 && actual[index] === 0) {
            missingPixels += 1;
            if (!near(actual, x, y)) missingBeyondTolerancePixels += 1;
          }
          if (actual[index] !== 0 && expected[index] === 0) {
            excessPixels += 1;
            if (!near(expected, x, y)) excessBeyondTolerancePixels += 1;
          }
        }
      }
      return {
        image: { width: imageWidth, height: imageHeight },
        hull,
        roi: { x0, x1, y0, y1 },
        expectedPixels,
        actualPixels,
        missingPixels,
        excessPixels,
        missingBeyondTolerancePixels,
        excessBeyondTolerancePixels,
      };
    },
    {
      base64: screenshot?.toString("base64"),
      maskBase64: occupancyMask ? Buffer.from(occupancyMask.bytes).toString("base64") : undefined,
      maskWidth: occupancyMask?.width ?? 0,
      maskHeight: occupancyMask?.height ?? 0,
      corners: projectedCorners,
      tolerance: EDGE_TOLERANCE_PIXELS,
    },
  );
}

async function missionScreenshot(page: Page): Promise<Buffer> {
  const pending = page.waitForEvent("download", { timeout: 15_000 });
  await page.keyboard.press("F2");
  const download = await pending;
  const path = await download.path();
  const failure = await download.failure();
  if (failure !== null || path === null) {
    throw new Error(`edge oracle screenshot failed: ${failure ?? "missing file"}`);
  }
  return readFile(path);
}

function fixtureOccupancyMask(
  attachment: TerrainDiagnosticAttachment,
  projectedCorners: readonly (readonly [number, number])[],
  bounds: ReturnType<typeof cubeBounds>,
) {
  const mask = new Uint8Array(attachment.width * attachment.height);
  const x0 = Math.max(0, Math.floor(Math.min(...projectedCorners.map((point) => point[0]))) - 8);
  const x1 = Math.min(
    attachment.width,
    Math.ceil(Math.max(...projectedCorners.map((point) => point[0]))) + 8,
  );
  const y0 = Math.max(0, Math.floor(Math.min(...projectedCorners.map((point) => point[1]))) - 8);
  const y1 = Math.min(
    attachment.height,
    Math.ceil(Math.max(...projectedCorners.map((point) => point[1]))) + 8,
  );
  const epsilon = 0.025;
  for (let y = y0; y < y1; y += 1) {
    for (let x = x0; x < x1; x += 1) {
      const world = attachment.pixel(x, y).worldMetres;
      if (
        world !== undefined &&
        world.every(
          (value, axis) =>
            value >= bounds.minimum[axis]! - epsilon && value <= bounds.maximum[axis]! + epsilon,
        )
      ) {
        mask[x + y * attachment.width] = 1;
      }
    }
  }
  return { width: attachment.width, height: attachment.height, bytes: mask };
}

async function runVoxelEdges(context: ScenarioContext, arguments_: readonly string[]) {
  const options = parseOptions(arguments_);
  const world = await startWorldStack(context, {
    fixture: {
      prefix: "voxels-voxel-edges-",
      source: options.source,
      spawnVoxels: options.coordinateSpace === "negative" ? [-4_208, -6_082] : [4_208, 6_082],
      spawnPillarHeightVoxels: 1,
      spawnPillarRadiusVoxels: 1,
      spawnProtectionRadiusVoxels: 1,
      dayLengthSeconds: 0,
      dayFractionAtUnixEpoch: 0.42,
      weatherCycleSeconds: 0,
      weatherFractionAtUnixEpoch: 0.08,
      cloudVelocityMetresPerSecond: [0, 0],
    },
    service: { metal: options.source === "terrain-diffusion-30m" },
    web: { buildProfile: options.buildProfile },
  });
  const browser = await BrowserCapability.start(context, {
    warningPattern: FAILURE,
    launch: chromeWebGpuLaunchOptions(),
  });
  const viewport = await browser.open({
    url: world.url,
    label: "voxel-edges",
    viewport: options.viewport,
    deviceScaleFactor: options.deviceScaleFactor,
    ...world.clientRoute,
  });
  const { engine, page } = viewport;
  const contract = await engine.ready(90_000);
  await waitForSettledWorld(engine, "voxel edge world did not settle");
  await engine.setCameraLook(0, 0);
  const origin = await moveForward(page, engine, 10);
  const groundY = Math.round(
    (snapshotValue(origin, "cameraY") - contract.semantics.playerEyeHeightMetres) *
      VOXELS_PER_METRE,
  );
  const materialId = await collectMaterial(
    engine,
    origin,
    groundY,
    contract.semantics.editCubeVolumeVoxels * 2,
  );
  const cameraX = Math.round(snapshotValue(origin, "cameraX") * VOXELS_PER_METRE);
  const cameraZ = Math.round(snapshotValue(origin, "cameraZ") * VOXELS_PER_METRE);
  const first = [cameraX - 5, groundY + 35, cameraZ - 50] as const;
  const second = [first[0] + 10, first[1], first[2]] as const;
  await submitDigIfSolid(engine, first, "first edge fixture clearance");
  await submitDigIfSolid(engine, second, "second edge fixture clearance");
  await place(engine, first, materialId, "first edge fixture cube");
  await place(engine, second, materialId, "second edge fixture cube");
  await waitForSettledWorld(engine, "voxel edge fixtures did not settle");
  await engine.setSpectator(true);
  const bounds = cubeBounds(first, second);
  const target = bounds.minimum.map(
    (minimum, axis) => (minimum + (bounds.maximum[axis] ?? minimum)) * 0.5,
  ) as [number, number, number];
  // Keep the crosshair and its hovered-voxel wireframe off the fixture. They are valid gameplay
  // UI, but including either in the silhouette mask would make the geometry oracle measure the
  // selection overlay rather than the voxel mesh.
  const aligned = await aimAt(engine, await engine.snapshot(), [
    target[0],
    target[1] + 1.5,
    target[2],
  ]);
  await engine.wait(300);
  await engine.setDiagnosticSky([255, 0, 255]);
  const screenshot = await page.screenshot();
  await context.artifacts.write(
    "Analytic adjacent-cube edge fixture",
    "voxel-edges.png",
    screenshot,
    "image/png",
  );
  const physicalWidth = options.viewport.width * options.deviceScaleFactor;
  const physicalHeight = options.viewport.height * options.deviceScaleFactor;
  const projected = projectBox(aligned, bounds, physicalWidth, physicalHeight);
  const analysis = await analyzeProjectedBox(page, screenshot, projected);
  const reproductionPng = await missionScreenshot(page);
  const reproductionText = readPngText(reproductionPng, "voxels.reproduction");
  if (reproductionText === undefined) {
    throw new Error("edge oracle reproduction capture omitted metadata");
  }
  const reproduction = JSON.parse(reproductionText) as {
    camera: {
      eyeMetres: [number, number, number];
      velocityMetresPerSecond: [number, number, number];
      yawRadians: number;
      pitchRadians: number;
      headingDegrees: number;
      grounded: boolean;
      locomotion: string;
    };
  };
  let randomState = 0x8f70_1a2b;
  const random = () => {
    randomState ^= randomState << 13;
    randomState ^= randomState >>> 17;
    randomState ^= randomState << 5;
    return (randomState >>> 0) / 0x1_0000_0000;
  };
  const randomizedAnalyses = [];
  for (let pose = 0; pose < options.randomizedPoses; pose += 1) {
    const azimuth = random() * Math.PI * 2;
    const radius = 6 + random() * 6;
    const eye: [number, number, number] = [
      Math.fround(target[0] + Math.sin(azimuth) * radius),
      Math.fround(target[1] + (random() - 0.5) * 0.8),
      Math.fround(target[2] + Math.cos(azimuth) * radius),
    ];
    // Keep the fixture above the horizon and the crosshair below it. This prevents either ground
    // pixels or the hovered-voxel overlay from entering the analytic silhouette ROI.
    const lookTarget: Vector3 = [target[0], target[1] - 1.5, target[2]];
    const dx = lookTarget[0] - eye[0];
    const dy = lookTarget[1] - eye[1];
    const dz = lookTarget[2] - eye[2];
    reproduction.camera.eyeMetres = eye;
    reproduction.camera.velocityMetresPerSecond = [0, 0, 0];
    reproduction.camera.yawRadians = Math.fround(Math.atan2(dx, -dz));
    reproduction.camera.pitchRadians = Math.fround(Math.atan2(dy, Math.hypot(dx, dz)));
    reproduction.camera.headingDegrees = (reproduction.camera.yawRadians * 180) / Math.PI;
    reproduction.camera.grounded = false;
    reproduction.camera.locomotion = "spectator";
    const poseSnapshot = await engine.applyReproduction(JSON.stringify(reproduction));
    await engine.waitForFrameAfter(snapshotValue(poseSnapshot, "frameSequence"));
    const poseProjected = projectBox(poseSnapshot, bounds, physicalWidth, physicalHeight);
    const poseCapture = await missionScreenshot(page);
    const poseAttachment = readTerrainDiagnosticAttachment(poseCapture);
    const poseAnalysis = await analyzeProjectedBox(
      page,
      undefined,
      poseProjected,
      fixtureOccupancyMask(poseAttachment, poseProjected, bounds),
    );
    randomizedAnalyses.push({
      pose,
      eye,
      yaw: reproduction.camera.yawRadians,
      pitch: reproduction.camera.pitchRadians,
      ...poseAnalysis,
    });
  }
  const violations: string[] = [];
  for (const [label, result] of [
    ["baseline", analysis],
    ...randomizedAnalyses.map((result) => [`pose ${result.pose}`, result] as const),
  ] as const) {
    if (result.missingBeyondTolerancePixels !== 0) {
      violations.push(
        `${label}: ${result.missingBeyondTolerancePixels} analytic cube pixels were missing beyond ${EDGE_TOLERANCE_PIXELS}px tolerance`,
      );
    }
    if (result.excessBeyondTolerancePixels !== 0) {
      violations.push(
        `${label}: ${result.excessBeyondTolerancePixels} rendered pixels exceeded the analytic cube beyond ${EDGE_TOLERANCE_PIXELS}px tolerance`,
      );
    }
  }
  const report = {
    ok: violations.length === 0,
    commit: execFileSync("git", ["rev-parse", "HEAD"], { encoding: "utf8" }).trim(),
    dirty: execFileSync("git", ["status", "--porcelain"], { encoding: "utf8" }).trim() !== "",
    source: options.source,
    browser: browser.version,
    options,
    camera: {
      position: cameraPosition(aligned),
      yaw: snapshotValue(aligned, "yaw"),
      pitch: snapshotValue(aligned, "pitch"),
    },
    fixture: { first, second, bounds, materialId },
    tolerancePixels: EDGE_TOLERANCE_PIXELS,
    analysis,
    randomizedAnalyses,
    violations,
  };
  await context.artifacts.writeJson("Voxel edge oracle report", "report.json", report);
  browser.assertHealthy();
  if (!report.ok) throw new Error(`voxel edge violations: ${violations.join("; ")}`);
  return {
    summary: "Adjacent voxel cubes matched their analytic projected silhouette.",
    metrics: analysis,
    details: report,
  };
}

export default defineScenario({
  id: "voxel-edges",
  kind: "validation",
  summary: "Compares adjacent rendered cubes against an analytic projected-box edge oracle.",
  uses: {
    world: true,
    browser: true,
    viewport: "browser",
    screenshots: true,
    metrics: true,
    rust: true,
  },
  timeoutMs: 600_000,
  run: runVoxelEdges,
});
