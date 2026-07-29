import { describe, expect, it } from "vite-plus/test";
import { SNAPSHOT, SNAPSHOT_SCHEMA_VERSION } from "./engine.ts";
import { PlayerPresentationInvariantState } from "./player-presentation-recorder.ts";

function snapshot(values: Partial<Record<keyof typeof SNAPSHOT, number>>): number[] {
  const result = Array.from<number>({ length: SNAPSHOT.droppedSamples + 1 }).fill(0);
  result[SNAPSHOT.schemaVersion] = SNAPSHOT_SCHEMA_VERSION;
  for (const [field, value] of Object.entries(values)) {
    result[SNAPSHOT[field as keyof typeof SNAPSHOT]] = value;
  }
  return result;
}

function playableFrame(
  frameSequence: number,
  values: Partial<Record<keyof typeof SNAPSHOT, number>> = {},
): number[] {
  return snapshot({
    frameSequence,
    terrainReady: 1,
    virtualTerrainMode: 2,
    virtualTerrainExactCoreComplete: 1,
    virtualTerrainExactCoreRequiredLeaves: 9,
    virtualTerrainExactCoreCurrentCoverage: 9,
    virtualTerrainOwnerlessRoots: 0,
    virtualTerrainGpuEncodingOverflowFlags: 0,
    virtualTerrainPublishedPages: 16,
    virtualTerrainPublishedExactPages: 9,
    virtualTerrainPublishedMinimumLevel: 0,
    virtualTerrainPublishedMaximumLevel: 4,
    virtualTerrainPublishedExactLodDiscontinuities: 0,
    virtualTerrainCutFingerprintLow24: 1,
    virtualTerrainPresentedSnapshotGenerationLow24: 2,
    virtualTerrainPresentedSnapshotFingerprintLow24: 3,
    virtualTerrainPresentedSnapshotMatchesCut: 1,
    virtualTerrainDesiredEnvelopeComplete: 1,
    virtualTerrainDesiredEnvelopeFingerprintLow24: 5,
    virtualTerrainDesiredSafetyLeaves: 9,
    virtualTerrainDesiredHorizonRoots: 4,
    virtualTerrainDesiredLocusMinimumLeafX: -2,
    virtualTerrainDesiredLocusMinimumLeafZ: -2,
    virtualTerrainDesiredLocusMaximumLeafExclusiveX: 2,
    virtualTerrainDesiredLocusMaximumLeafExclusiveZ: 2,
    virtualTerrainCommittedEnvelopeFingerprintLow24: 5,
    virtualTerrainCommittedSafetyLeaves: 9,
    virtualTerrainCommittedSafetyCoverage: 9,
    virtualTerrainCommittedHorizonRoots: 4,
    virtualTerrainCommittedHorizonCoverage: 4,
    virtualTerrainCommittedLocusMinimumLeafX: -2,
    virtualTerrainCommittedLocusMinimumLeafZ: -2,
    virtualTerrainCommittedLocusMaximumLeafExclusiveX: 2,
    virtualTerrainCommittedLocusMaximumLeafExclusiveZ: 2,
    presentedCameraInsideCommittedEnvelope: 1,
    ...values,
  });
}

describe("continuous player presentation invariant state", () => {
  it("observes cold start before terrainReady without treating empty shadow state as a regression", () => {
    const state = new PlayerPresentationInvariantState();

    expect(
      state.observe(
        snapshot({
          frameSequence: 1,
          terrainReady: 0,
          virtualTerrainMode: 1,
          virtualTerrainGpuMatchesCpuCut: 0,
        }),
        "startup",
      ),
    ).toBeUndefined();
    expect(state.observedFrames).toBe(1);
    expect(state.firstPlayableFrameSequence).toBeUndefined();
  });

  it("allows an in-flight candidate mismatch while the published bank remains exact and valid", () => {
    const state = new PlayerPresentationInvariantState();

    const violation = state.observe(
      playableFrame(7, {
        virtualTerrainGpuMatchesCpuCut: 0,
        virtualTerrainRequestedPages: 8,
        presentationGateActive: 1,
      }),
      "travel",
    );

    expect(violation).toBeUndefined();
    expect(state.firstPlayableFrameSequence).toBe(7);
    expect(state.trace()[0]?.gpu.candidateMatchesCpu).toBe(false);
  });

  it("fails on a one-frame exact-coverage regression before a later frame can hide it", () => {
    const state = new PlayerPresentationInvariantState();
    expect(state.observe(playableFrame(10), "pedestal-step")).toBeUndefined();

    const violation = state.observe(
      playableFrame(11, {
        virtualTerrainPublishedExactPages: 0,
        virtualTerrainPublishedMinimumLevel: 3,
      }),
      "pedestal-step",
    );

    expect(violation?.frameSequence).toBe(11);
    expect(violation?.reasons).toContain("published cut lost every exact L0 page");
    expect(violation?.reasons).toContain("published minimum LOD regressed to L3");
    expect(violation?.trace.map((frame) => frame.frameSequence)).toEqual([10, 11]);
  });

  it("rejects an empty virtual frame even when canonical chunks are not the surface owner", () => {
    const state = new PlayerPresentationInvariantState();
    expect(state.observe(playableFrame(14), "travel")).toBeUndefined();

    const violation = state.observe(
      playableFrame(15, {
        terrainReady: 0,
        canonicalLatticePresented: 0,
        collisionImmediateResident: 7,
        collisionImmediateRequired: 8,
        virtualTerrainPublishedPages: 0,
        virtualTerrainPublishedExactPages: 0,
        virtualTerrainCutFingerprintLow24: 0,
        virtualTerrainPresentedSnapshotGenerationLow24: 0,
        virtualTerrainPresentedSnapshotFingerprintLow24: 0,
        virtualTerrainPresentedSnapshotMatchesCut: 0,
      }),
      "travel",
    );

    expect(violation?.reasons).toContain("published cut became empty");
    expect(violation?.reasons).toContain("presented GPU bank does not match the published CPU cut");
  });

  it("fails when the committed target loses exact core ownership for one frame", () => {
    const state = new PlayerPresentationInvariantState();
    expect(state.observe(playableFrame(20), "travel")).toBeUndefined();

    const violation = state.observe(
      playableFrame(21, {
        virtualTerrainExactCoreCurrentCoverage: 8,
      }),
      "travel",
    );

    expect(violation?.reasons).toEqual(["committed target has exact core 8/9, complete=true"]);
  });

  it("allows an overlapping desired envelope to stream while the presented camera stays certified", () => {
    const state = new PlayerPresentationInvariantState();

    const violation = state.observe(
      playableFrame(22, {
        virtualTerrainDesiredEnvelopeFingerprintLow24: 6,
        virtualTerrainExactCoreCurrentCoverage: 8,
      }),
      "pedestal-step",
    );

    expect(violation).toBeUndefined();
  });

  it("fails when the presented camera leaves the committed exact envelope", () => {
    const state = new PlayerPresentationInvariantState();

    const violation = state.observe(
      playableFrame(23, {
        presentedCameraInsideCommittedEnvelope: 0,
      }),
      "travel",
    );

    expect(violation?.reasons).toContain(
      "presented gameplay camera is outside the committed exact envelope",
    );
  });

  it("fails when the camera outruns the complete committed terrain horizon for one frame", () => {
    const state = new PlayerPresentationInvariantState();

    const violation = state.observe(
      playableFrame(24, {
        virtualTerrainPresentedCoverageGapFrames: 1,
      }),
      "spectator-travel",
    );

    expect(violation?.reasons).toContain(
      "renderer accumulated 1 frame(s) outside its complete terrain horizon",
    );
  });

  it("fails on one unsafe published GPU bank without confusing it with an in-flight candidate", () => {
    const state = new PlayerPresentationInvariantState();
    expect(state.observe(playableFrame(30), "dig")).toBeUndefined();

    const violation = state.observe(
      playableFrame(31, {
        virtualTerrainGpuMatchesCpuCut: 0,
        virtualTerrainGpuEncodingOverflowFlags: 2,
        virtualTerrainPresentedSnapshotMatchesCut: 0,
      }),
      "dig",
    );

    expect(violation?.reasons).toEqual([
      "GPU encoding overflow flags are 2",
      "presented GPU bank does not match the published CPU cut",
    ]);
    expect(violation?.trace.at(-1)?.gpu.candidateMatchesCpu).toBe(false);
  });

  it("fails when the committed presentation envelope becomes incomplete for one frame", () => {
    const state = new PlayerPresentationInvariantState();
    expect(state.observe(playableFrame(40), "travel-settle")).toBeUndefined();

    const violation = state.observe(
      playableFrame(41, {
        virtualTerrainCommittedSafetyCoverage: 8,
        virtualTerrainCommittedHorizonCoverage: 3,
      }),
      "travel-settle",
    );

    expect(violation?.reasons).toEqual([
      "committed exact safety coverage is 8/9",
      "committed horizon coverage is 3/4",
    ]);
  });

  it("deduplicates repeated snapshots and bounds retained trace history", () => {
    const state = new PlayerPresentationInvariantState(2);

    expect(state.observe(snapshot({ frameSequence: 1 }), "startup")).toBeUndefined();
    expect(state.observe(snapshot({ frameSequence: 1 }), "startup")).toBeUndefined();
    expect(state.observe(snapshot({ frameSequence: 2 }), "startup")).toBeUndefined();
    expect(state.observe(snapshot({ frameSequence: 3 }), "startup")).toBeUndefined();

    expect(state.observedFrames).toBe(3);
    expect(state.trace().map((frame) => frame.frameSequence)).toEqual([2, 3]);
  });
});
