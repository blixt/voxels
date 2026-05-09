import { expect, test } from "bun:test";

import { VOXEL_RPG_VERIFICATION_BUDGETS } from "../scripts/lib/voxel-rpg-budgets.ts";

test("RPG verification budgets centralize required artifact and performance gates", () => {
  expect(VOXEL_RPG_VERIFICATION_BUDGETS).toEqual({
    minRouteSamples: 100,
    minLiveForwardSamples: 120,
    minViewCount: 1,
    minLodIterations: 1,
    maxP95FrameMs: 16.67,
    maxFrameMs: 50,
    maxFpsErrorRatio: 0.10,
    requireArtifacts: true,
  });
});
