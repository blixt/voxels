export interface FarFieldColumnSample {
  readonly surfaceY: number;
  readonly surfaceMaterial: number;
  readonly waterTopY: number | null;
  readonly waterMaterial: number | null;
}

export interface FarFieldSurfaceSource {
  readonly palette: readonly number[];
  sampleFarFieldColumn(worldX: number, worldZ: number): FarFieldColumnSample | null;
}
