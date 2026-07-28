import { FRAME_SAMPLE_WIDTH, SNAPSHOT, snapshotValue } from "./engine.ts";

const FRAME_SAMPLE_START = SNAPSHOT.droppedSamples + 1;

export interface FrameSample {
  readonly intervalMs: number;
  readonly cpuMs: number;
  readonly simulationMs: number;
  readonly streamingMs: number;
  readonly renderSubmissionMs: number;
  readonly frameId: number;
  readonly renderCullMs: number;
  readonly renderEncodeMs: number;
  readonly renderSubmitMs: number;
  readonly testedSlices: number;
  readonly selectedSlices: number;
  readonly streamRemoteMs: number;
  readonly streamPlanMs: number;
  readonly streamMeshMs: number;
  readonly streamPublishMs: number;
  readonly streamVirtualTerrainMs: number;
  readonly streamPresenceMs: number;
  readonly streamInterestMs: number;
  readonly streamSchedulerUpdateMs: number;
  readonly streamSchedulerAdmitMs: number;
  readonly streamCollisionInterestMs: number;
  readonly streamEnclosedInterestMs: number;
}

function required(values: readonly number[], index: number): number {
  const value = values[index];
  if (value === undefined) throw new Error(`frame sample omitted value ${index}`);
  return value;
}

/** Decodes timing history without making any claim about visible renderer correctness. */
export function frameSamples(snapshot: readonly number[]): FrameSample[] {
  const samples: FrameSample[] = [];
  const count = snapshotValue(snapshot, "sampleCount");
  for (let index = 0; index < count; index += 1) {
    const start = FRAME_SAMPLE_START + index * FRAME_SAMPLE_WIDTH;
    samples.push({
      intervalMs: required(snapshot, start),
      cpuMs: required(snapshot, start + 1),
      simulationMs: required(snapshot, start + 2),
      streamingMs: required(snapshot, start + 3),
      renderSubmissionMs: required(snapshot, start + 4),
      frameId: required(snapshot, start + 5),
      renderCullMs: required(snapshot, start + 6),
      renderEncodeMs: required(snapshot, start + 7),
      renderSubmitMs: required(snapshot, start + 8),
      testedSlices: required(snapshot, start + 9),
      selectedSlices: required(snapshot, start + 10),
      streamRemoteMs: required(snapshot, start + 11),
      streamPlanMs: required(snapshot, start + 12),
      streamMeshMs: required(snapshot, start + 13),
      streamPublishMs: required(snapshot, start + 14),
      streamVirtualTerrainMs: required(snapshot, start + 15),
      streamPresenceMs: required(snapshot, start + 16),
      streamInterestMs: required(snapshot, start + 17),
      streamSchedulerUpdateMs: required(snapshot, start + 18),
      streamSchedulerAdmitMs: required(snapshot, start + 19),
      streamCollisionInterestMs: required(snapshot, start + 20),
      streamEnclosedInterestMs: required(snapshot, start + 21),
    });
  }
  return samples;
}
