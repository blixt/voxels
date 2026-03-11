import { hashUint32 } from "./math.ts";

const HASH_UINT32_INV = 1 / 0xffffffff;
const NOISE_X_PRIME = 374761393;
const NOISE_Y_PRIME = 668265263;
const NOISE_Z_PRIME = 2147483647;
const FBM_OCTAVE_STEP = 977;
const FBM_TWO_OCTAVE_NORMALIZER = 1 / 1.5;
const FBM_THREE_OCTAVE_NORMALIZER = 1 / 1.75;
const FBM_FOUR_OCTAVE_NORMALIZER = 1 / 1.875;
const FBM_FIVE_OCTAVE_NORMALIZER = 1 / 1.9375;

export function valueNoise2D(x: number, z: number, seed: number): number {
  const x0 = Math.floor(x);
  const z0 = Math.floor(z);
  const tx = x - x0;
  const tz = z - z0;
  const xBase = Math.imul(x0, NOISE_X_PRIME);
  const zBase = Math.imul(z0, NOISE_Y_PRIME);
  const cornerBase = xBase + zBase + seed;
  const h00 = hashUint32(cornerBase) * HASH_UINT32_INV;
  const h10 = hashUint32(cornerBase + NOISE_X_PRIME) * HASH_UINT32_INV;
  const h01 = hashUint32(cornerBase + NOISE_Y_PRIME) * HASH_UINT32_INV;
  const h11 = hashUint32(cornerBase + NOISE_X_PRIME + NOISE_Y_PRIME) * HASH_UINT32_INV;

  const sx = tx * tx * (3 - 2 * tx);
  const sz = tz * tz * (3 - 2 * tz);
  const nx0 = h00 + (h10 - h00) * sx;
  const nx1 = h01 + (h11 - h01) * sx;
  return nx0 + (nx1 - nx0) * sz;
}

export function fbm2D2(x: number, z: number, seed: number): number {
  const octave0 = valueNoise2D(x, z, seed);
  const octave1 = valueNoise2D(x * 2, z * 2, seed + FBM_OCTAVE_STEP);
  return (octave0 + octave1 * 0.5) * FBM_TWO_OCTAVE_NORMALIZER;
}

export function fbm2D3(x: number, z: number, seed: number): number {
  const octave0 = valueNoise2D(x, z, seed);
  const octave1 = valueNoise2D(x * 2, z * 2, seed + FBM_OCTAVE_STEP);
  const octave2 = valueNoise2D(x * 4, z * 4, seed + FBM_OCTAVE_STEP * 2);
  return (octave0 + octave1 * 0.5 + octave2 * 0.25) * FBM_THREE_OCTAVE_NORMALIZER;
}

export function fbm2D4(x: number, z: number, seed: number): number {
  const octave0 = valueNoise2D(x, z, seed);
  const octave1 = valueNoise2D(x * 2, z * 2, seed + FBM_OCTAVE_STEP);
  const octave2 = valueNoise2D(x * 4, z * 4, seed + FBM_OCTAVE_STEP * 2);
  const octave3 = valueNoise2D(x * 8, z * 8, seed + FBM_OCTAVE_STEP * 3);
  return (octave0 + octave1 * 0.5 + octave2 * 0.25 + octave3 * 0.125) * FBM_FOUR_OCTAVE_NORMALIZER;
}

export function fbm2D5(x: number, z: number, seed: number): number {
  const octave0 = valueNoise2D(x, z, seed);
  const octave1 = valueNoise2D(x * 2, z * 2, seed + FBM_OCTAVE_STEP);
  const octave2 = valueNoise2D(x * 4, z * 4, seed + FBM_OCTAVE_STEP * 2);
  const octave3 = valueNoise2D(x * 8, z * 8, seed + FBM_OCTAVE_STEP * 3);
  const octave4 = valueNoise2D(x * 16, z * 16, seed + FBM_OCTAVE_STEP * 4);
  return (
    octave0
    + octave1 * 0.5
    + octave2 * 0.25
    + octave3 * 0.125
    + octave4 * 0.0625
  ) * FBM_FIVE_OCTAVE_NORMALIZER;
}

export function fbm2D(x: number, z: number, octaves: number, seed: number): number {
  if (octaves === 2) {
    return fbm2D2(x, z, seed);
  }
  if (octaves === 3) {
    return fbm2D3(x, z, seed);
  }
  if (octaves === 4) {
    return fbm2D4(x, z, seed);
  }
  if (octaves === 5) {
    return fbm2D5(x, z, seed);
  }
  let amplitude = 1;
  let frequency = 1;
  let total = 0;
  let sum = 0;
  for (let octave = 0; octave < octaves; octave += 1) {
    total += valueNoise2D(x * frequency, z * frequency, seed + octave * FBM_OCTAVE_STEP) * amplitude;
    sum += amplitude;
    amplitude *= 0.5;
    frequency *= 2;
  }
  return total / sum;
}

export function hashNoise3D(x: number, y: number, z: number, seed: number): number {
  return hashUint32(
    Math.imul(x, NOISE_X_PRIME)
      + Math.imul(y, NOISE_Y_PRIME)
      + Math.imul(z, NOISE_Z_PRIME)
      + seed,
  ) * HASH_UINT32_INV;
}
