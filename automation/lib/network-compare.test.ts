import { describe, expect, it } from "vite-plus/test";
import { compareNetworkBenchmarks, type NetworkBenchmarkResult } from "./network-compare.ts";

function result(
  viewport: number,
  coverage: number,
  bytes: number,
  down = 50,
): NetworkBenchmarkResult {
  return {
    schemaVersion: 4,
    environment: {
      platform: "darwin 25.0.0",
      cpu: "Apple M3 Max",
      logicalCpus: 16,
      chrome: "150.0.7871.115",
      node: "v24.18.0",
    },
    world: { source: "terrain-diffusion-30m" },
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
    repetitions: 5,
    git: { commit: "fixture" },
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
    expect(comparison.cold_spawn?.viewportMedianMs).toEqual({
      before: 1_000,
      after: 800,
      delta: -200,
      percent: -20,
    });
    expect(comparison.cold_spawn?.fullCoverageMedianMs.delta).toBe(100);
    expect(comparison.cold_spawn?.viewportWorldBytes.percent).toBe(-20);
    expect(comparison.cold_spawn?.fullCoverageBytes.percent).toBe(-20);
  });

  it("rejects incomparable link profiles", () => {
    expect(() =>
      compareNetworkBenchmarks(result(1_000, 1_500, 10_000), result(800, 1_600, 8_000, 25)),
    ).toThrow("link mismatch");
  });

  it("rejects incomparable fixtures", () => {
    const candidate = result(800, 1_600, 8_000);
    (candidate.fixture as { streamingWalkMetres: number }).streamingWalkMetres = 50;
    expect(() => compareNetworkBenchmarks(result(1_000, 1_500, 10_000), candidate)).toThrow(
      "fixture mismatch",
    );
  });

  it("rejects incomplete or expanded scenario sets", () => {
    const baseline = result(1_000, 1_500, 10_000);
    const missing = result(800, 1_600, 8_000);
    const coldSpawn = missing.summary.cold_spawn;
    if (coldSpawn === undefined) throw new Error("test fixture lacks cold_spawn");
    missing.summary.resident_walk = structuredClone(coldSpawn);
    expect(() => compareNetworkBenchmarks(missing, baseline)).toThrow(
      "scenario set mismatch: cold_spawn, resident_walk versus cold_spawn",
    );

    const expanded = result(800, 1_600, 8_000);
    const expandedColdSpawn = expanded.summary.cold_spawn;
    if (expandedColdSpawn === undefined) throw new Error("test fixture lacks cold_spawn");
    expanded.summary.streaming_walk = structuredClone(expandedColdSpawn);
    expect(() => compareNetworkBenchmarks(baseline, expanded)).toThrow(
      "scenario set mismatch: cold_spawn versus cold_spawn, streaming_walk",
    );
  });

  it("rejects different run counts, world sources, and environments", () => {
    const baseline = result(1_000, 1_500, 10_000);

    const fewerRuns = result(800, 1_600, 8_000);
    fewerRuns.repetitions = 1;
    expect(() => compareNetworkBenchmarks(baseline, fewerRuns)).toThrow(
      "repetitions mismatch: 5 versus 1",
    );

    const procedural = result(800, 1_600, 8_000);
    (procedural.world as { source: string }).source = "procedural-v16";
    expect(() => compareNetworkBenchmarks(baseline, procedural)).toThrow("world mismatch");

    const otherCpu = result(800, 1_600, 8_000);
    (otherCpu.environment as { cpu: string }).cpu = "Other CPU";
    expect(() => compareNetworkBenchmarks(baseline, otherCpu)).toThrow("environment mismatch");
  });
});
