import { expect, test } from "bun:test";

import { fnv1a, packRgba } from "../src/engine/math.ts";
import { CLEAR_COLOR_RGBA } from "../src/engine/render-constants.ts";
import { renderReferenceImage } from "../src/engine/reference-render.ts";
import type { SceneCameraPreset } from "../src/engine/types.ts";
import { VoxelWorld } from "../src/engine/world.ts";

const TEST_CAMERA: SceneCameraPreset = {
  target: [2, 2, 2],
  yaw: 0,
  pitch: 0,
  distance: 10,
  zoom: 2,
};

function createTestWorld(): VoxelWorld {
  return new VoxelWorld(
    { width: 4, height: 4, depth: 4 },
    4,
    [0, packRgba(220, 90, 70), packRgba(80, 140, 220)],
  );
}

function countCoveredPixels(pixels: Uint8ClampedArray): number {
  let count = 0;
  for (let index = 0; index < pixels.length; index += 4) {
    const r = pixels[index]!;
    const g = pixels[index + 1]!;
    const b = pixels[index + 2]!;
    if (r !== CLEAR_COLOR_RGBA[0] || g !== CLEAR_COLOR_RGBA[1] || b !== CLEAR_COLOR_RGBA[2]) {
      count += 1;
    }
  }
  return count;
}

function measureCoverageBounds(pixels: Uint8ClampedArray, width: number, height: number) {
  let minX = width;
  let minY = height;
  let maxX = -1;
  let maxY = -1;
  for (let y = 0; y < height; y += 1) {
    for (let x = 0; x < width; x += 1) {
      const index = (y * width + x) * 4;
      const r = pixels[index]!;
      const g = pixels[index + 1]!;
      const b = pixels[index + 2]!;
      if (r === CLEAR_COLOR_RGBA[0] && g === CLEAR_COLOR_RGBA[1] && b === CLEAR_COLOR_RGBA[2]) {
        continue;
      }
      minX = Math.min(minX, x);
      minY = Math.min(minY, y);
      maxX = Math.max(maxX, x);
      maxY = Math.max(maxY, y);
    }
  }
  return { minX, minY, maxX, maxY };
}

test("reference renderer clears empty worlds to the shared background color", () => {
  const world = createTestWorld();
  const image = renderReferenceImage(world, TEST_CAMERA, 16, 16);

  for (let index = 0; index < image.pixels.length; index += 4) {
    expect(image.pixels[index + 0]).toBe(CLEAR_COLOR_RGBA[0]);
    expect(image.pixels[index + 1]).toBe(CLEAR_COLOR_RGBA[1]);
    expect(image.pixels[index + 2]).toBe(CLEAR_COLOR_RGBA[2]);
    expect(image.pixels[index + 3]).toBe(255);
  }
});

test("reference renderer produces visible coverage for a single voxel", () => {
  const world = createTestWorld();
  world.setVoxel(1, 1, 1, 1);

  const image = renderReferenceImage(world, TEST_CAMERA, 32, 32);
  const covered = countCoveredPixels(image.pixels);

  expect(covered).toBeGreaterThan(0);
});

test("rear voxels are fully occluded by a nearer voxel in orthographic views", () => {
  const frontOnly = createTestWorld();
  frontOnly.setVoxel(1, 1, 1, 1);

  const withRearVoxel = createTestWorld();
  withRearVoxel.setVoxel(1, 1, 1, 1);
  withRearVoxel.setVoxel(2, 1, 1, 2);

  const frontImage = renderReferenceImage(frontOnly, TEST_CAMERA, 32, 32);
  const layeredImage = renderReferenceImage(withRearVoxel, TEST_CAMERA, 32, 32);

  const frontChecksum = fnv1a(new Uint8Array(frontImage.pixels.buffer.slice(0)));
  const layeredChecksum = fnv1a(new Uint8Array(layeredImage.pixels.buffer.slice(0)));

  expect(layeredChecksum).toBe(frontChecksum);
});

test("stacking a voxel above the base voxel expands the silhouette upward", () => {
  const baseWorld = createTestWorld();
  baseWorld.setVoxel(1, 1, 1, 1);

  const stackedWorld = createTestWorld();
  stackedWorld.setVoxel(1, 1, 1, 1);
  stackedWorld.setVoxel(1, 2, 1, 2);

  const baseImage = renderReferenceImage(baseWorld, TEST_CAMERA, 32, 32);
  const stackedImage = renderReferenceImage(stackedWorld, TEST_CAMERA, 32, 32);
  const baseBounds = measureCoverageBounds(baseImage.pixels, baseImage.width, baseImage.height);
  const stackedBounds = measureCoverageBounds(stackedImage.pixels, stackedImage.width, stackedImage.height);

  expect(countCoveredPixels(stackedImage.pixels)).toBeGreaterThan(countCoveredPixels(baseImage.pixels));
  expect(stackedBounds.minY).toBeLessThan(baseBounds.minY);
});
