import { expect, test } from "bun:test";

import {
  DEFAULT_VOXEL_RPG_VERIFICATION_PROFILE_ID,
  resolveVoxelRpgVerificationProfile,
  VOXEL_RPG_VERIFICATION_BUDGETS,
  VOXEL_RPG_VERIFICATION_PROFILES,
} from "../scripts/lib/voxel-rpg-budgets.ts";

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

test("RPG verification profiles expose render, evidence, and hitch budgets", () => {
  const profile = resolveVoxelRpgVerificationProfile(DEFAULT_VOXEL_RPG_VERIFICATION_PROFILE_ID);

  expect(profile).toBe(VOXEL_RPG_VERIFICATION_PROFILES["rpg-render-gate"]);
  expect(profile.renderThresholds).toBe(VOXEL_RPG_VERIFICATION_BUDGETS);
  expect(profile.evidence).toMatchObject({
    requireGeneratedAt: true,
    requireMatchingCommit: true,
  });
  expect(profile.hitches).toMatchObject({
    hitchFrameMs: 50,
    maxHitchFrames: 0,
    maxMovementHitchFrames: 0,
    maxSettleHitchFrames: 0,
    maxLodWorkHitchFrames: 0,
  });
  expect(resolveVoxelRpgVerificationProfile("rpg-render-smoke").renderThresholds.minLiveForwardSamples).toBe(20);
  expect(() => resolveVoxelRpgVerificationProfile("unknown" as never)).toThrow(
    "Unknown RPG verification profile",
  );
});
