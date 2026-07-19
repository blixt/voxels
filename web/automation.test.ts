import { describe, expect, it } from "vite-plus/test";
import {
  AUTOMATION_CONTRACT_VERSION,
  FRAME_SAMPLE_WIDTH,
  GPU_SAMPLE_WIDTH,
  SNAPSHOT,
  SNAPSHOT_FIELD_NAMES,
  SNAPSHOT_SCHEMA_VERSION,
  assertAutomationContract,
  assertSnapshotSchema,
  parseAutomationContract,
} from "./automation.ts";

describe("engine automation contract", () => {
  it("covers every fixed snapshot field exactly once", () => {
    expect(SNAPSHOT_FIELD_NAMES).toHaveLength(SNAPSHOT.droppedSamples + 1);
    expect(new Set(SNAPSHOT_FIELD_NAMES).size).toBe(SNAPSHOT_FIELD_NAMES.length);
    expect(SNAPSHOT_FIELD_NAMES[SNAPSHOT.cameraX]).toBe("cameraX");
    expect(SNAPSHOT_FIELD_NAMES[SNAPSHOT.arenaPages]).toBe("arenaPages");
    expect(SNAPSHOT_FIELD_NAMES[SNAPSHOT.targetVoxelZ]).toBe("targetVoxelZ");
    expect(SNAPSHOT_FIELD_NAMES[SNAPSHOT.schemaVersion]).toBe("schemaVersion");
  });

  it("parses and validates the Rust-owned envelope", () => {
    const contract = parseAutomationContract(
      [
        AUTOMATION_CONTRACT_VERSION,
        SNAPSHOT_SCHEMA_VERSION,
        FRAME_SAMPLE_WIDTH,
        GPU_SAMPLE_WIDTH,
        SNAPSHOT_FIELD_NAMES.join(","),
      ].join("\n"),
    );
    expect(() => assertAutomationContract(contract)).not.toThrow();
  });

  it("rejects drift in versions, widths, names, and snapshot values", () => {
    const valid = {
      version: AUTOMATION_CONTRACT_VERSION,
      snapshotVersion: SNAPSHOT_SCHEMA_VERSION,
      frameSampleWidth: FRAME_SAMPLE_WIDTH,
      gpuSampleWidth: GPU_SAMPLE_WIDTH,
      snapshotFields: SNAPSHOT_FIELD_NAMES,
    };
    expect(() => assertAutomationContract({ ...valid, version: valid.version + 1 })).toThrow();
    expect(() =>
      assertAutomationContract({ ...valid, frameSampleWidth: valid.frameSampleWidth + 1 }),
    ).toThrow();
    expect(() =>
      assertAutomationContract({
        ...valid,
        snapshotFields: valid.snapshotFields.with(0, "movedCamera"),
      }),
    ).toThrow();

    const snapshot = Array<number>(SNAPSHOT.droppedSamples + 1).fill(0);
    snapshot[SNAPSHOT.schemaVersion] = SNAPSHOT_SCHEMA_VERSION;
    expect(assertSnapshotSchema(snapshot)).toBe(snapshot);
    snapshot[SNAPSHOT.schemaVersion] = SNAPSHOT_SCHEMA_VERSION + 1;
    expect(() => assertSnapshotSchema(snapshot)).toThrow();
  });
});
