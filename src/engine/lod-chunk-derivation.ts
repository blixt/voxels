export interface LodSourceMaterialSample {
  material: number;
  complete: boolean;
}

export interface LodGeneratedTopMaterialSample {
  bucketIndex: number;
  material: number;
}

export interface LodChunkDerivationInput {
  chunkSize: number;
  originX: number;
  originY: number;
  originZ: number;
  level: number;
  stride: number;
  useGeneratedTopMaterial: boolean;
  sampleSourceMaterial(ox: number, oy: number, oz: number): LodSourceMaterialSample;
  sampleGeneratedTopMaterial(
    ox: number,
    oz: number,
    minOy: number,
    maxOyExclusive: number,
  ): LodGeneratedTopMaterialSample | null;
  sampleGeneratedColumnMaterials(
    ox: number,
    oz: number,
    startOy: number,
    endOyInclusive: number,
  ): ArrayLike<number>;
  sampleSurfaceYRange(ox: number, oz: number): { minY: number; maxY: number };
  isOutputColumnCoveredByFiner(ox: number, oz: number): boolean;
}

export interface LodChunkDerivationResult {
  data: Uint16Array;
  solidCount: number;
  solidBounds: { min: [number, number, number]; max: [number, number, number] } | null;
  sourceComplete: boolean;
  skippedFinerCoverage: boolean;
}

export function deriveLodChunkData(input: LodChunkDerivationInput): LodChunkDerivationResult {
  const cs = input.chunkSize;
  const chunkArea = cs * cs;
  const data = new Uint16Array(cs * chunkArea);
  let solidCount = 0;
  let minX = cs;
  let minY = cs;
  let minZ = cs;
  let maxX = 0;
  let maxY = 0;
  let maxZ = 0;
  let sourceComplete = true;
  let skippedFinerCoverage = false;

  const recordMaterial = (ox: number, oy: number, oz: number, material: number): void => {
    data[ox + oy * cs + oz * chunkArea] = material;
    solidCount += 1;
    if (ox < minX) minX = ox;
    if (oy < minY) minY = oy;
    if (oz < minZ) minZ = oz;
    if (ox + 1 > maxX) maxX = ox + 1;
    if (oy + 1 > maxY) maxY = oy + 1;
    if (oz + 1 > maxZ) maxZ = oz + 1;
  };

  const fillColumn = (ox: number, oz: number): void => {
    if (input.isOutputColumnCoveredByFiner(ox, oz)) {
      skippedFinerCoverage = true;
      return;
    }
    const shellPaddingY = input.stride * 3;
    if (input.useGeneratedTopMaterial) {
      const topBucket = input.sampleGeneratedTopMaterial(ox, oz, 0, cs);
      if (topBucket) {
        recordMaterial(ox, topBucket.bucketIndex, oz, topBucket.material);
      }
      return;
    }

    const range = input.sampleSurfaceYRange(ox, oz);
    const startOy = Math.max(0, Math.floor((range.minY - shellPaddingY - input.originY) / input.stride));
    const endOy = Math.min(cs - 1, Math.floor((range.maxY - input.originY) / input.stride));
    if (startOy > endOy) {
      return;
    }

    if (input.level === 1 || !input.useGeneratedTopMaterial) {
      let sourceColumnComplete = true;
      for (let oy = endOy; oy >= startOy; oy -= 1) {
        const source = input.sampleSourceMaterial(ox, oy, oz);
        if (!source.complete) {
          sourceColumnComplete = false;
        }
        if (source.material !== 0) {
          recordMaterial(ox, oy, oz, source.material);
          return;
        }
      }
      if (sourceColumnComplete) {
        return;
      }
      sourceComplete = false;
    }

    const materials = input.sampleGeneratedColumnMaterials(ox, oz, startOy, endOy);
    for (let oy = endOy; oy >= startOy; oy -= 1) {
      const material = materials[oy - startOy] ?? 0;
      if (material !== 0) {
        recordMaterial(ox, oy, oz, material);
        return;
      }
    }
  };

  for (let oz = 0; oz < cs; oz += 1) {
    for (let ox = 0; ox < cs; ox += 1) {
      fillColumn(ox, oz);
    }
  }

  return {
    data,
    solidCount,
    solidBounds: solidCount === 0 ? null : { min: [minX, minY, minZ], max: [maxX, maxY, maxZ] },
    sourceComplete,
    skippedFinerCoverage,
  };
}
