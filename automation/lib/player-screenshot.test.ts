import { describe, expect, it } from "vite-plus/test";
import { summarizeSurfaceCutAdjacency } from "./player-screenshot.ts";

const SURFACE_Y = -2_147_483_648;

function page(level: number, x: number, z: number) {
  return {
    level,
    coord: [x, SURFACE_Y, z] as const,
  };
}

describe("player screenshot surface-cut audit", () => {
  it("accepts an incremental level transition", () => {
    const summary = summarizeSurfaceCutAdjacency([page(0, 1, 0), page(1, 1, 0)]);

    expect(summary.adjacentEdges).toBe(1);
    expect(summary.maximumLevelDelta).toBe(1);
    expect(summary.discontinuousEdges).toBe(0);
  });

  it("detects a coarse page placed directly beside exact terrain", () => {
    const summary = summarizeSurfaceCutAdjacency([page(0, 3, 0), page(2, 1, 0)]);

    expect(summary.adjacentEdges).toBe(1);
    expect(summary.maximumLevelDelta).toBe(2);
    expect(summary.discontinuousEdges).toBe(1);
    expect(summary.discontinuitySamples[0]).toContain("L0@3,0 <> L2@1,0");
  });

  it("limits the audit to the player vicinity when requested", () => {
    const summary = summarizeSurfaceCutAdjacency(
      [page(0, 103, 0), page(2, 26, 0)],
      [0, 0, 0],
      12.8,
    );

    expect(summary.adjacentEdges).toBe(0);
    expect(summary.discontinuousEdges).toBe(0);
  });
});
