import { describe, expect, it } from "vite-plus/test";
import { compareNetworkBenchmarks } from "./network-benchmark-compare.mjs";

function result(viewport, coverage, bytes, down = 50) {
  return {
    schemaVersion: 2,
    browserSnapshotSchema: 18,
    fixture: { version: 2, streamingWalkMetres: 35 },
    protocol: { name: "VXWP", version: 4, resultCompression: { codec: "brotli" } },
    link: {
      roundTripLatencyMs: 40,
      downstreamMegabitsPerSecond: down,
      upstreamMegabitsPerSecond: 10,
      quantumBytes: 16_384,
      upstreamMaxQueuedBytes: 100_000,
      downstreamMaxQueuedBytes: 500_000,
    },
    summary: {
      cold_spawn: {
        viewportFullyInformedMs: { median: viewport, max: viewport * 1.1 },
        fullCoverageSettledMs: { median: coverage },
        bytesAtViewportInformed: { medianWorldDownstream: bytes * 0.8 },
        bytesAtFullCoverage: { medianTotal: bytes },
      },
    },
  };
}

describe("network benchmark comparison", () => {
  it("reports signed time and byte deltas", () => {
    const comparison = compareNetworkBenchmarks(
      result(1_000, 1_500, 10_000),
      result(800, 1_600, 8_000),
    );
    expect(comparison.cold_spawn.viewportMedianMs).toEqual({
      before: 1_000,
      after: 800,
      delta: -200,
      percent: -20,
    });
    expect(comparison.cold_spawn.fullCoverageMedianMs.delta).toBe(100);
    expect(comparison.cold_spawn.viewportWorldBytes.percent).toBe(-20);
    expect(comparison.cold_spawn.fullCoverageBytes.percent).toBe(-20);
  });

  it("rejects incomparable link profiles", () => {
    expect(() =>
      compareNetworkBenchmarks(result(1_000, 1_500, 10_000), result(800, 1_600, 8_000, 25)),
    ).toThrow("link profile mismatch for downstreamMegabitsPerSecond");
  });

  it("rejects incomparable fixtures", () => {
    const candidate = result(800, 1_600, 8_000);
    candidate.fixture.streamingWalkMetres = 50;
    expect(() => compareNetworkBenchmarks(result(1_000, 1_500, 10_000), candidate)).toThrow(
      "fixture mismatch",
    );
  });

  it("rejects incomplete or expanded scenario sets", () => {
    const baseline = result(1_000, 1_500, 10_000);
    const missing = result(800, 1_600, 8_000);
    missing.summary.resident_walk = structuredClone(missing.summary.cold_spawn);
    expect(() => compareNetworkBenchmarks(missing, baseline)).toThrow(
      "scenario set mismatch: cold_spawn, resident_walk versus cold_spawn",
    );

    const expanded = result(800, 1_600, 8_000);
    expanded.summary.streaming_walk = structuredClone(expanded.summary.cold_spawn);
    expect(() => compareNetworkBenchmarks(baseline, expanded)).toThrow(
      "scenario set mismatch: cold_spawn versus cold_spawn, streaming_walk",
    );
  });
});
