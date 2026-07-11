import { describe, expect, it } from "vite-plus/test";
import { decodeEngineStats, formatEngineSnapshot } from "./ui.ts";

describe("decodeEngineStats", () => {
  it("decodes the stable Rust snapshot protocol", () => {
    const stats = decodeEngineStats([
      1, 2, 3, 0.25, -0.1, 1, 12_345, 9, 24, 48, 16, 12, 2, 4.5, 8, 3, 30, 12.5,
    ]);

    expect(stats.position).toEqual([1, 2, 3]);
    expect(stats.grounded).toBe(true);
    expect(stats.residentQuads).toBe(12_345);
    expect(stats.fps).toBe(80);
    expect(stats.farResident).toBe(30);
  });

  it("defaults absent and invalid counters without leaking NaN", () => {
    const stats = decodeEngineStats([Number.NaN]);
    expect(stats.position).toEqual([0, 0, 0]);
    expect(stats.fps).toBe(0);
    expect(formatEngineSnapshot(stats)).not.toContain("NaN");
  });
});
