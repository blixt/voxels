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
});
