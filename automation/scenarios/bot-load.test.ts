import { describe, expect, it } from "vite-plus/test";
import { parseBotHarnessReport, settlePopulationSampling } from "./bot-load.ts";

function report(): unknown {
  return {
    schemaVersion: 5,
    wallTimeMs: 1_000,
    connectionCount: 1,
    posesSent: 30,
    maxVisiblePlayers: 0,
    editsAccepted: 1,
    editsSubmitted: 1,
    editsRejected: 0,
    editConflicts: 0,
    editAuthorityRejections: 0,
    authorityRejectionReasons: {},
    mutationsCommitted: 1,
    behaviors: { explorer: 1 },
    reports: [
      {
        chunkLatency: { samples: 1, p95Ms: 4 },
        editLatency: { samples: 1, p95Ms: 5 },
        maxVisiblePlayers: 0,
        resyncs: 0,
        protocolErrors: 0,
        errorSamples: [],
        finalOutboundRateBytesPerSecond: 32_768,
        maxOutboundRateBytesPerSecond: 65_536,
        traffic: {
          receivedPayloadBytes: 4_096,
          maxReceivedFrameBytes: 1_024,
          receivedByKind: { "14": { payloadBytes: 512 } },
        },
      },
    ],
  };
}

describe("native bot report boundary", () => {
  it("accepts the current complete report schema", () => {
    expect(parseBotHarnessReport(report()).schemaVersion).toBe(5);
  });

  it("rejects version drift and partial client reports", () => {
    expect(() => parseBotHarnessReport({ ...(report() as object), schemaVersion: 6 })).toThrow(
      "incompatible report",
    );
    expect(() =>
      parseBotHarnessReport({ ...(report() as object), reports: [{ protocolErrors: 0 }] }),
    ).toThrow("incompatible report");
  });
});

describe("bot load process settlement", () => {
  it("settles both samplers without masking the bot process failure", async () => {
    const primary = new Error("bot failed");
    const secondary = new Error("observer stopped");
    let resolveSamples: ((value: string) => void) | undefined;
    let rejectObserver: ((reason: Error) => void) | undefined;
    const samples = new Promise<string>((resolve) => {
      resolveSamples = resolve;
    });
    const observer = new Promise<number>((_resolve, reject) => {
      rejectObserver = reject;
    });
    const settled = settlePopulationSampling(Promise.reject(primary), samples, observer, () => {
      resolveSamples?.("samples stopped");
      rejectObserver?.(secondary);
    });

    await expect(settled).rejects.toBe(primary);
  });

  it("reports sampler failures after a successful bot process", async () => {
    const samplingFailure = new Error("sampling failed");
    await expect(
      settlePopulationSampling(
        Promise.resolve(),
        Promise.reject(samplingFailure),
        Promise.resolve(1),
        () => {},
      ),
    ).rejects.toBe(samplingFailure);
  });
});
