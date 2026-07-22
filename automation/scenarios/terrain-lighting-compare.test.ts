import { describe, expect, it } from "vite-plus/test";
import { lightingRatios, type LightingSummary } from "./terrain-lighting-compare.ts";

function summary(overrides: Partial<LightingSummary> = {}): LightingSummary {
  return {
    blockMean: 1,
    blockP10: 1,
    blockP90: 2,
    blockP90P10: 2,
    blockP90ToP10: 4,
    coarseGradientRms: 0.5,
    coarseGradientRelativeRms: 0.25,
    ...overrides,
  };
}

describe("terrain lighting comparison", () => {
  it("computes finite ratios from a measurable reference", () => {
    expect(
      lightingRatios(
        summary(),
        summary({
          blockP90P10: 4,
          blockP90ToP10: 2,
          coarseGradientRms: 1,
          coarseGradientRelativeRms: 0.125,
        }),
      ),
    ).toEqual({
      blockP90P10: 2,
      blockP90ToP10: 0.5,
      coarseGradientRms: 2,
      coarseGradientRelativeRms: 0.5,
    });
  });

  it("rejects references without a measurable signal", () => {
    expect(() => lightingRatios(summary({ blockP90P10: 0 }), summary())).toThrow(
      "no measurable block contrast",
    );
  });

  it("rejects non-finite candidate metrics", () => {
    expect(() =>
      lightingRatios(summary(), summary({ coarseGradientRms: Number.POSITIVE_INFINITY })),
    ).toThrow("candidate terrain lighting coarse gradient must be finite");
  });
});
