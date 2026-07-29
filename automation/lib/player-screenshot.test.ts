import { describe, expect, it } from "vite-plus/test";
import {
  assertPlayerScreenshotMetadata,
  summarizeSurfaceCutAdjacency,
} from "./player-screenshot.ts";

const SURFACE_Y = -2_147_483_648;

function page(level: number, x: number, z: number) {
  return {
    level,
    coord: [x, SURFACE_Y, z] as const,
  };
}

describe("player screenshot surface-cut audit", () => {
  const reproducibleMetadata = {
    schema: "voxels.reproduction.v3",
    camera: { eyeMetres: [1, 2, 3] },
    presentation: {
      publishedClientView: {
        presentationSerial: "3",
        viewRevision: "9",
        terrainRequest: 4,
        terrainGeneration: 7,
        revisionDigest: "1234567890abcdef",
      },
      selectedCutFingerprint: "0000000000001234",
      terrainHandleSnapshot: {
        generation: "7",
        presentationFingerprint: "0000000000005678",
        cutFingerprint: "0000000000001234",
        matchesPublishedCut: true,
      },
      selectedCut: {
        kind: "virtualTerrain",
        cut: {
          renderable: true,
          feedbackOverflow: false,
          selectionOverflow: false,
          traversalOverflow: false,
          selectedPages: [],
          refinementRoots: [],
        },
      },
      virtualTerrain: {
        mode: "visible",
        exactSurfaceDomain: {
          complete: true,
          requiredLeaves: 4,
          fingerprint: "0123456789abcdef",
          currentExactCoverage: 4,
          oracleExactCoverage: 4,
          coreComplete: true,
          coreRequiredLeaves: 2,
          coreFingerprint: "1111111111111111",
          coreCurrentCoverage: 2,
          coreOracleCoverage: 2,
          predictionComplete: true,
          predictionRequiredLeaves: 4,
          predictionFingerprint: "0123456789abcdef",
          predictionCurrentCoverage: 4,
          predictionOracleCoverage: 4,
        },
      },
    },
    attachments: {
      terrainPixelOwnership: {
        schema: "voxels.terrain-pixel-ownership.v1",
      },
    },
  };

  it("requires the exact same-frame motion domain in screenshot metadata", () => {
    expect(() => assertPlayerScreenshotMetadata(reproducibleMetadata)).not.toThrow();
    expect(() =>
      assertPlayerScreenshotMetadata({
        ...reproducibleMetadata,
        presentation: {
          ...reproducibleMetadata.presentation,
          virtualTerrain: {
            ...reproducibleMetadata.presentation.virtualTerrain,
            exactSurfaceDomain: {
              complete: false,
              requiredLeaves: 2,
              fingerprint: "1111111111111111",
              currentExactCoverage: 2,
              oracleExactCoverage: 2,
              coreComplete: true,
              coreRequiredLeaves: 2,
              coreFingerprint: "1111111111111111",
              coreCurrentCoverage: 2,
              coreOracleCoverage: 2,
              predictionComplete: false,
              predictionRequiredLeaves: 0,
              predictionFingerprint: "0000000000000000",
              predictionCurrentCoverage: 0,
              predictionOracleCoverage: 0,
            },
          },
        },
      }),
    ).not.toThrow();
    expect(() =>
      assertPlayerScreenshotMetadata({
        ...reproducibleMetadata,
        presentation: {
          ...reproducibleMetadata.presentation,
          terrainHandleSnapshot: {
            ...reproducibleMetadata.presentation.terrainHandleSnapshot,
            cutFingerprint: "0000000000009999",
          },
        },
      }),
    ).toThrow();
    expect(() =>
      assertPlayerScreenshotMetadata({
        ...reproducibleMetadata,
        presentation: {
          ...reproducibleMetadata.presentation,
          virtualTerrain: {
            ...reproducibleMetadata.presentation.virtualTerrain,
            exactSurfaceDomain: {
              ...reproducibleMetadata.presentation.virtualTerrain.exactSurfaceDomain,
              complete: false,
              requiredLeaves: 0,
              fingerprint: "0000000000000000",
              currentExactCoverage: 0,
              oracleExactCoverage: 0,
              coreComplete: false,
              coreRequiredLeaves: 0,
              coreFingerprint: "0000000000000000",
              coreCurrentCoverage: 0,
              coreOracleCoverage: 0,
              predictionComplete: false,
              predictionRequiredLeaves: 0,
              predictionFingerprint: "0000000000000000",
              predictionCurrentCoverage: 0,
              predictionOracleCoverage: 0,
            },
          },
        },
      }),
    ).toThrow();
    expect(() =>
      assertPlayerScreenshotMetadata({
        ...reproducibleMetadata,
        presentation: {
          ...reproducibleMetadata.presentation,
          virtualTerrain: {
            ...reproducibleMetadata.presentation.virtualTerrain,
            exactSurfaceDomain: {
              ...reproducibleMetadata.presentation.virtualTerrain.exactSurfaceDomain,
              currentExactCoverage: 5,
            },
          },
        },
      }),
    ).toThrow();
    expect(() =>
      assertPlayerScreenshotMetadata({
        ...reproducibleMetadata,
        presentation: {
          ...reproducibleMetadata.presentation,
          virtualTerrain: {},
        },
      }),
    ).toThrow();
  });

  it("accepts an incremental level transition", () => {
    const summary = summarizeSurfaceCutAdjacency([page(0, 1, 0), page(1, 1, 0)]);

    expect(summary.adjacentEdges).toBe(1);
    expect(summary.maximumLevelDelta).toBe(1);
    expect(summary.discontinuousEdges).toBe(0);
  });

  it("detects a coarse page placed directly beside exact terrain", () => {
    const summary = summarizeSurfaceCutAdjacency([page(0, 3, 0), page(2, 1, 0)]);

    expect(summary.adjacentEdges).toBe(1);
    expect(summary.maximumLevelDelta).toBe(2);
    expect(summary.discontinuousEdges).toBe(1);
    expect(summary.discontinuitySamples[0]).toContain("L0@3,0 <> L2@1,0");
  });

  it("limits the audit to the player vicinity when requested", () => {
    const summary = summarizeSurfaceCutAdjacency(
      [page(0, 103, 0), page(2, 26, 0)],
      [0, 0, 0],
      12.8,
    );

    expect(summary.adjacentEdges).toBe(0);
    expect(summary.discontinuousEdges).toBe(0);
  });
});
