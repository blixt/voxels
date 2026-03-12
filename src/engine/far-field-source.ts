import type { GeneratedChunkRenderSummary } from "./generated-chunk-render-summary.ts";

export interface FarFieldColumnSample {
  readonly surfaceY: number;
  readonly surfaceMaterial: number;
  readonly waterTopY: number | null;
  readonly waterMaterial: number | null;
}

export interface FarFieldSource {
  readonly palette: readonly number[];
  sampleFarFieldColumn(worldX: number, worldZ: number): FarFieldColumnSample | null;
  getFarFieldChunkSummary(cx: number, cy: number, cz: number): GeneratedChunkRenderSummary | null;
}
