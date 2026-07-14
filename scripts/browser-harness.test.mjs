import { describe, expect, it } from "vite-plus/test";
import {
  assertSnapshotSchema,
  isBrowserConsoleFailure,
  SNAPSHOT,
  SNAPSHOT_SCHEMA_VERSION,
} from "./browser-harness.mjs";

describe("browser snapshot schema", () => {
  it("rejects stale layouts before interpreting named fields", () => {
    expect([
      SNAPSHOT.cameraX,
      SNAPSHOT.cameraY,
      SNAPSHOT.cameraZ,
      SNAPSHOT.yaw,
      SNAPSHOT.pitch,
    ]).toEqual([0, 1, 2, 3, 4]);

    const current = [];
    current[SNAPSHOT.schemaVersion] = SNAPSHOT_SCHEMA_VERSION;
    expect(assertSnapshotSchema(current)).toBe(current);

    const stale = [];
    stale[SNAPSHOT.schemaVersion] = SNAPSHOT_SCHEMA_VERSION - 1;
    expect(() => assertSnapshotSchema(stale)).toThrow(
      `snapshot schema ${SNAPSHOT_SCHEMA_VERSION - 1} does not match ${SNAPSHOT_SCHEMA_VERSION}`,
    );
  });

  it("always rejects console errors and filters warnings narrowly", () => {
    const warnings = /webgpu|sqlite/i;
    expect(isBrowserConsoleFailure("error", "render loop stopped", warnings)).toBe(true);
    expect(isBrowserConsoleFailure("warning", "WebGPU validation", warnings)).toBe(true);
    expect(isBrowserConsoleFailure("warning", "development hint", warnings)).toBe(false);
    expect(isBrowserConsoleFailure("log", "sqlite", warnings)).toBe(false);
  });
});
