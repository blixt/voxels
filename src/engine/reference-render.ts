import { buildCameraMatrices } from "./camera.ts";
import { dotVec3, fnv1a, normalizeVec3, unpackRgba } from "./math.ts";
import { CLEAR_COLOR_RGBA, LIGHT_DIRECTION, LIGHTING_TERMS } from "./render-constants.ts";
import type {
  RenderValidationArtifacts,
  RenderValidationMetrics,
  SceneCameraPreset,
} from "./types.ts";
import { VoxelWorld } from "./world.ts";

interface ImageBuffer {
  width: number;
  height: number;
  pixels: Uint8ClampedArray;
}

export function renderReferenceImage(
  world: VoxelWorld,
  camera: SceneCameraPreset,
  width: number,
  height: number,
): ImageBuffer {
  const pixels = new Uint8ClampedArray(width * height * 4);
  pixels.fill(0);
  for (let index = 0; index < width * height; index += 1) {
    pixels.set(CLEAR_COLOR_RGBA, index * 4);
  }
  const depthBuffer = new Float32Array(width * height);
  depthBuffer.fill(Number.POSITIVE_INFINITY);

  const matrices = buildCameraMatrices(camera, width / height);
  const lightVector = normalizeVec3([-LIGHT_DIRECTION[0], -LIGHT_DIRECTION[1], -LIGHT_DIRECTION[2]]);

  for (let z = 0; z < world.depth; z += 1) {
    for (let y = 0; y < world.height; y += 1) {
      for (let x = 0; x < world.width; x += 1) {
        const material = world.getVoxel(x, y, z);
        if (material === 0) {
          continue;
        }
        const [r, g, b] = unpackRgba(world.getPaletteColor(material));
        for (const face of EXPOSED_FACES) {
          if (world.getVoxel(x + face.normal[0], y + face.normal[1], z + face.normal[2]) !== 0) {
            continue;
          }
          const directional = Math.max(dotVec3(face.normal, lightVector), 0);
          const hemi = face.normal[1] * 0.5 + 0.5;
          const lighting = LIGHTING_TERMS[0] + LIGHTING_TERMS[1] * directional + LIGHTING_TERMS[2] * hemi;
          const color: [number, number, number, number] = [
            clampByte(r * lighting),
            clampByte(g * lighting),
            clampByte(b * lighting),
            255,
          ];
          const vertices = face.corners.map((corner) => projectVertex(
            [x + corner[0], y + corner[1], z + corner[2]],
            matrices.viewProjection,
            width,
            height,
          ));
          rasterizeTriangle(vertices[0]!, vertices[1]!, vertices[2]!, color, pixels, depthBuffer, width, height);
          rasterizeTriangle(vertices[0]!, vertices[2]!, vertices[3]!, color, pixels, depthBuffer, width, height);
        }
      }
    }
  }

  return { width, height, pixels };
}

export function compareRenderedImages(
  actual: ImageBuffer,
  reference: ImageBuffer,
): {
  metrics: RenderValidationMetrics;
  artifacts: RenderValidationArtifacts;
} {
  return {
    metrics: measureImageDifference(actual, reference),
    artifacts: createValidationArtifacts(actual, reference),
  };
}

export function measureImageDifference(
  actual: ImageBuffer,
  reference: ImageBuffer,
): RenderValidationMetrics {
  if (actual.width !== reference.width || actual.height !== reference.height) {
    throw new Error("Rendered image size does not match reference image size");
  }

  let totalError = 0;
  let maxAbsoluteError = 0;
  let coverageMismatchCount = 0;

  for (let index = 0; index < actual.pixels.length; index += 4) {
    const actualR = actual.pixels[index]!;
    const actualG = actual.pixels[index + 1]!;
    const actualB = actual.pixels[index + 2]!;
    const referenceR = reference.pixels[index]!;
    const referenceG = reference.pixels[index + 1]!;
    const referenceB = reference.pixels[index + 2]!;

    const errorR = Math.abs(actualR - referenceR);
    const errorG = Math.abs(actualG - referenceG);
    const errorB = Math.abs(actualB - referenceB);
    const pixelError = errorR + errorG + errorB;
    totalError += pixelError;
    maxAbsoluteError = Math.max(maxAbsoluteError, errorR, errorG, errorB);

    const actualCovered = isCovered(actualR, actualG, actualB);
    const referenceCovered = isCovered(referenceR, referenceG, referenceB);
    if (actualCovered !== referenceCovered) {
      coverageMismatchCount += 1;
    }
  }

  const pixelCount = actual.width * actual.height;
  const meanAbsoluteError = totalError / (pixelCount * 3);
  const coverageMismatchRatio = coverageMismatchCount / pixelCount;
  const renderChecksum = fnv1a(new Uint8Array(actual.pixels.buffer.slice(0)));
  const referenceChecksum = fnv1a(new Uint8Array(reference.pixels.buffer.slice(0)));
  const visualPass = meanAbsoluteError <= 10 && maxAbsoluteError <= 64 && coverageMismatchRatio <= 0.06;

  return {
    meanAbsoluteError,
    maxAbsoluteError,
    coverageMismatchRatio,
    renderChecksum,
    referenceChecksum,
    visualPass,
  };
}

export function createValidationArtifacts(
  actual: ImageBuffer,
  reference: ImageBuffer,
): RenderValidationArtifacts {
  const diffPixels = new Uint8ClampedArray(actual.pixels.length);
  for (let index = 0; index < actual.pixels.length; index += 4) {
    const actualR = actual.pixels[index]!;
    const actualG = actual.pixels[index + 1]!;
    const actualB = actual.pixels[index + 2]!;
    const referenceR = reference.pixels[index]!;
    const referenceG = reference.pixels[index + 1]!;
    const referenceB = reference.pixels[index + 2]!;
    const pixelError = Math.abs(actualR - referenceR) + Math.abs(actualG - referenceG) + Math.abs(actualB - referenceB);
    const heat = clampByte((pixelError / 3) * 4);
    diffPixels[index + 0] = heat;
    diffPixels[index + 1] = isCovered(actualR, actualG, actualB) === isCovered(referenceR, referenceG, referenceB) ? 0 : 255;
    diffPixels[index + 2] = 255 - heat;
    diffPixels[index + 3] = 255;
  }

  return {
    actualDataUrl: imageToDataUrl(actual),
    referenceDataUrl: imageToDataUrl(reference),
    diffDataUrl: imageToDataUrl({ width: actual.width, height: actual.height, pixels: diffPixels }),
    width: actual.width,
    height: actual.height,
  };
}

function imageToDataUrl(image: ImageBuffer): string {
  const canvas = document.createElement("canvas");
  canvas.width = image.width;
  canvas.height = image.height;
  const context = canvas.getContext("2d");
  if (!context) {
    throw new Error("Unable to create 2D context for benchmark artifact generation");
  }
  const copy = new Uint8ClampedArray(image.pixels.length);
  copy.set(image.pixels);
  context.putImageData(new ImageData(copy, image.width, image.height), 0, 0);
  return canvas.toDataURL("image/png");
}

function isCovered(r: number, g: number, b: number): boolean {
  return Math.abs(r - CLEAR_COLOR_RGBA[0]) > 2 ||
    Math.abs(g - CLEAR_COLOR_RGBA[1]) > 2 ||
    Math.abs(b - CLEAR_COLOR_RGBA[2]) > 2;
}

interface ProjectedVertex {
  x: number;
  y: number;
  depth: number;
}

type FaceDefinition = {
  normal: [number, number, number];
  corners: [
    [number, number, number],
    [number, number, number],
    [number, number, number],
    [number, number, number]
  ];
};

const EXPOSED_FACES: FaceDefinition[] = [
  {
    normal: [1, 0, 0],
    corners: [
      [1, 0, 0],
      [1, 1, 0],
      [1, 1, 1],
      [1, 0, 1],
    ],
  },
  {
    normal: [-1, 0, 0],
    corners: [
      [0, 0, 0],
      [0, 0, 1],
      [0, 1, 1],
      [0, 1, 0],
    ],
  },
  {
    normal: [0, 1, 0],
    corners: [
      [0, 1, 0],
      [1, 1, 0],
      [1, 1, 1],
      [0, 1, 1],
    ],
  },
  {
    normal: [0, -1, 0],
    corners: [
      [0, 0, 0],
      [0, 0, 1],
      [1, 0, 1],
      [1, 0, 0],
    ],
  },
  {
    normal: [0, 0, 1],
    corners: [
      [0, 0, 1],
      [1, 0, 1],
      [1, 1, 1],
      [0, 1, 1],
    ],
  },
  {
    normal: [0, 0, -1],
    corners: [
      [0, 0, 0],
      [0, 1, 0],
      [1, 1, 0],
      [1, 0, 0],
    ],
  },
];

function clampByte(value: number): number {
  return Math.max(0, Math.min(255, Math.round(value)));
}

function projectVertex(
  point: [number, number, number],
  viewProjection: Float32Array,
  width: number,
  height: number,
): ProjectedVertex {
  const clipX = viewProjection[0]! * point[0] + viewProjection[4]! * point[1] + viewProjection[8]! * point[2] + viewProjection[12]!;
  const clipY = viewProjection[1]! * point[0] + viewProjection[5]! * point[1] + viewProjection[9]! * point[2] + viewProjection[13]!;
  const clipZ = viewProjection[2]! * point[0] + viewProjection[6]! * point[1] + viewProjection[10]! * point[2] + viewProjection[14]!;
  const clipW = viewProjection[3]! * point[0] + viewProjection[7]! * point[1] + viewProjection[11]! * point[2] + viewProjection[15]!;
  const invW = clipW !== 0 ? 1 / clipW : 1;
  const ndcX = clipX * invW;
  const ndcY = clipY * invW;
  const ndcZ = clipZ * invW;
  return {
    x: (ndcX * 0.5 + 0.5) * width,
    y: (0.5 - ndcY * 0.5) * height,
    depth: ndcZ,
  };
}

function rasterizeTriangle(
  a: ProjectedVertex,
  b: ProjectedVertex,
  c: ProjectedVertex,
  color: readonly [number, number, number, number],
  pixels: Uint8ClampedArray,
  depthBuffer: Float32Array,
  width: number,
  height: number,
): void {
  const minX = Math.max(0, Math.floor(Math.min(a.x, b.x, c.x)));
  const maxX = Math.min(width - 1, Math.ceil(Math.max(a.x, b.x, c.x)));
  const minY = Math.max(0, Math.floor(Math.min(a.y, b.y, c.y)));
  const maxY = Math.min(height - 1, Math.ceil(Math.max(a.y, b.y, c.y)));
  const area = edgeFunction(a, b, c.x, c.y);
  if (area === 0) {
    return;
  }

  for (let y = minY; y <= maxY; y += 1) {
    for (let x = minX; x <= maxX; x += 1) {
      const px = x + 0.5;
      const py = y + 0.5;
      const w0 = edgeFunction(b, c, px, py);
      const w1 = edgeFunction(c, a, px, py);
      const w2 = edgeFunction(a, b, px, py);
      const inside = area > 0
        ? w0 >= 0 && w1 >= 0 && w2 >= 0
        : w0 <= 0 && w1 <= 0 && w2 <= 0;
      if (!inside) {
        continue;
      }
      const alpha = w0 / area;
      const beta = w1 / area;
      const gamma = w2 / area;
      const depth = alpha * a.depth + beta * b.depth + gamma * c.depth;
      const index = y * width + x;
      if (depth >= depthBuffer[index]!) {
        continue;
      }
      depthBuffer[index] = depth;
      const offset = index * 4;
      pixels[offset + 0] = color[0];
      pixels[offset + 1] = color[1];
      pixels[offset + 2] = color[2];
      pixels[offset + 3] = color[3];
    }
  }
}

function edgeFunction(a: ProjectedVertex, b: ProjectedVertex, x: number, y: number): number {
  return (x - a.x) * (b.y - a.y) - (y - a.y) * (b.x - a.x);
}
