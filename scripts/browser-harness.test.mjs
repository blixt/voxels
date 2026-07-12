import { describe, expect, it } from "vite-plus/test";
import { assertSnapshotSchema, SNAPSHOT, SNAPSHOT_SCHEMA_VERSION } from "./browser-harness.mjs";

describe("browser snapshot schema", () => {
  it("rejects stale layouts before interpreting named fields", () => {
    const current = [];
    current[SNAPSHOT.schemaVersion] = SNAPSHOT_SCHEMA_VERSION;
    expect(assertSnapshotSchema(current)).toBe(current);

    const stale = [];
    stale[SNAPSHOT.schemaVersion] = SNAPSHOT_SCHEMA_VERSION - 1;
    expect(() => assertSnapshotSchema(stale)).toThrow(/snapshot schema 14 does not match 15/);
  });
});
