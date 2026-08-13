import { describe, expect, it } from "vite-plus/test";
import { parseBotHarnessReport } from "./bot-load.ts";

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
