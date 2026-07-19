import type { Page } from "playwright";
import { describe, expect, it } from "vite-plus/test";
import { EngineClient, SNAPSHOT, SNAPSHOT_SCHEMA_VERSION, snapshotValue } from "./engine.ts";
import {
  AUTOMATION_CONTRACT_VERSION,
  FRAME_SAMPLE_WIDTH,
  GPU_SAMPLE_WIDTH,
  SNAPSHOT_FIELD_NAMES,
} from "../../web/automation.ts";

function snapshot(values: Partial<Record<keyof typeof SNAPSHOT, number>>): number[] {
  const result = Array.from<number>({ length: SNAPSHOT.droppedSamples + 1 }).fill(0);
  result[SNAPSHOT.schemaVersion] = SNAPSHOT_SCHEMA_VERSION;
  for (const [field, value] of Object.entries(values)) {
    result[SNAPSHOT[field as keyof typeof SNAPSHOT]] = value;
  }
  return result;
}

function pageReturning(values: unknown[]): Page {
  return {
    waitForFunction: async () => {},
    waitForTimeout: (milliseconds: number) =>
      new Promise<void>((resolve) => setTimeout(resolve, milliseconds)),
    evaluate: async () => values.shift(),
  } as unknown as Page;
}

describe("typed engine client", () => {
  it("validates the contract and centralizes camera convergence", async () => {
    const initial = snapshot({ yaw: 0.1, pitch: 0.2 });
    const settled = snapshot({ yaw: 0.4, pitch: -0.1 });
    const page = pageReturning([
      {
        version: AUTOMATION_CONTRACT_VERSION,
        snapshotVersion: SNAPSHOT_SCHEMA_VERSION,
        frameSampleWidth: FRAME_SAMPLE_WIDTH,
        gpuSampleWidth: GPU_SAMPLE_WIDTH,
        snapshotFields: SNAPSHOT_FIELD_NAMES,
      },
      initial,
      undefined,
      settled,
    ]);
    const engine = new EngineClient(page);

    const result = await engine.setCameraLook(0.4, -0.1);

    expect(snapshotValue(result, "yaw")).toBe(0.4);
    expect(snapshotValue(result, "pitch")).toBe(-0.1);
  });

  it("reports named convergence failures", async () => {
    const latest = snapshot({ pendingJobs: 3 });
    const page = pageReturning([
      {
        version: AUTOMATION_CONTRACT_VERSION,
        snapshotVersion: SNAPSHOT_SCHEMA_VERSION,
        frameSampleWidth: FRAME_SAMPLE_WIDTH,
        gpuSampleWidth: GPU_SAMPLE_WIDTH,
        snapshotFields: SNAPSHOT_FIELD_NAMES,
      },
      latest,
    ]);
    const engine = new EngineClient(page);

    await expect(
      engine.waitForSnapshot(() => false, {
        timeoutMs: 1,
        intervalMs: 2,
        description: "queues remained busy",
      }),
    ).rejects.toThrow("queues remained busy");
  });

  it("enters spectator mode through the typed Rust boundary", async () => {
    const contract = {
      version: AUTOMATION_CONTRACT_VERSION,
      snapshotVersion: SNAPSHOT_SCHEMA_VERSION,
      frameSampleWidth: FRAME_SAMPLE_WIDTH,
      gpuSampleWidth: GPU_SAMPLE_WIDTH,
      snapshotFields: SNAPSHOT_FIELD_NAMES,
    };
    const page = pageReturning([
      contract,
      true,
      snapshot({ spectatorActive: 1 }),
      false,
      snapshot({ spectatorActive: 0 }),
    ]);
    const engine = new EngineClient(page);
    await engine.ready();

    expect(snapshotValue(await engine.setSpectator(true), "spectatorActive")).toBe(1);
    expect(snapshotValue(await engine.setSpectator(false), "spectatorActive")).toBe(0);
  });
});
