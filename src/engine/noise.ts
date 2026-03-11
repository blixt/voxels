import { hashUint32 } from "./math.ts";

function smoothstep(value: number): number {
  return value * value * (3 - 2 * value);
}

function lerp(a: number, b: number, t: number): number {
  return a + (b - a) * t;
}

export function valueNoise2D(x: number, z: number, seed: number): number {
  const x0 = Math.floor(x);
  const z0 = Math.floor(z);
  const tx = x - x0;
  const tz = z - z0;

  const h00 = hashUint32(x0 * 374761393 + z0 * 668265263 + seed) / 0xffffffff;
  const h10 = hashUint32((x0 + 1) * 374761393 + z0 * 668265263 + seed) / 0xffffffff;
  const h01 = hashUint32(x0 * 374761393 + (z0 + 1) * 668265263 + seed) / 0xffffffff;
  const h11 = hashUint32((x0 + 1) * 374761393 + (z0 + 1) * 668265263 + seed) / 0xffffffff;

  const sx = smoothstep(tx);
  const sz = smoothstep(tz);
  const nx0 = lerp(h00, h10, sx);
  const nx1 = lerp(h01, h11, sx);
  return lerp(nx0, nx1, sz);
}

export function fbm2D(x: number, z: number, octaves: number, seed: number): number {
  let amplitude = 1;
  let frequency = 1;
  let total = 0;
  let sum = 0;
  for (let octave = 0; octave < octaves; octave += 1) {
    total += valueNoise2D(x * frequency, z * frequency, seed + octave * 977) * amplitude;
    sum += amplitude;
    amplitude *= 0.5;
    frequency *= 2;
  }
  return total / sum;
}

export function hashNoise3D(x: number, y: number, z: number, seed: number): number {
  return hashUint32(
    x * 374761393
      + y * 668265263
      + z * 2147483647
      + seed,
  ) / 0xffffffff;
}
