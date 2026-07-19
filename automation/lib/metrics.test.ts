import { describe, expect, it } from "vite-plus/test";
import { numericSummary, percentile } from "./metrics.ts";

describe("shared harness metrics", () => {
  it("uses nearest-rank percentiles consistently", () => {
    const values = Array.from({ length: 100 }, (_, index) => index + 1);
    expect(percentile(values, 0.5)).toBe(50);
    expect(percentile(values, 0.95)).toBe(95);
    expect(numericSummary(values).max).toBe(100);
  });

  it("reports an explicit empty distribution", () => {
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
});
