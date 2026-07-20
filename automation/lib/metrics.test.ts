import { describe, expect, it } from "vite-plus/test";
import {
  externalProcessCpuPercent,
  numericSummary,
  percentile,
  percentileOrNull,
  rounded,
} from "./metrics.ts";

describe("shared harness metrics", () => {
  it("uses nearest-rank percentiles consistently", () => {
    const values = Array.from({ length: 100 }, (_, index) => index + 1);
    expect(percentile(values, 0.5)).toBe(50);
    expect(percentile(values, 0.95)).toBe(95);
    expect(numericSummary(values).max).toBe(100);
  });

  it("reports an explicit empty distribution", () => {
    expect(percentileOrNull([], 0.95)).toBeNull();
    expect(numericSummary([])).toEqual({
      samples: 0,
      min: 0,
      p50: 0,
      p95: 0,
      p99: 0,
      max: 0,
      mean: 0,
    });
  });

  it("rejects invalid distribution and rounding parameters", () => {
    expect(() => percentile([1], -0.1)).toThrow("between zero and one");
    expect(() => percentile([1], 1.1)).toThrow("between zero and one");
    expect(() => rounded(Number.NaN)).toThrow("must be finite");
    expect(() => rounded(1, 13)).toThrow("between zero and twelve");
  });

  it("excludes the complete benchmark process tree from host contention", () => {
    expect(
      externalProcessCpuPercent(
        [
          { pid: 10, parentPid: 1, cpuPercent: 4 },
          { pid: 20, parentPid: 10, cpuPercent: 90 },
          { pid: 21, parentPid: 20, cpuPercent: 120 },
          { pid: 30, parentPid: 1, cpuPercent: 350 },
          { pid: 31, parentPid: 30, cpuPercent: 50 },
        ],
        10,
      ),
    ).toBe(400);
  });
});
