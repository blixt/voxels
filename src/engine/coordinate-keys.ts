import type { ChunkCoordinate, RenderSummaryRegionCoordinate } from "./types.ts";

export function formatChunkCoordinateKey(cx: number, cy: number, cz: number): string {
  return `${cx}:${cy}:${cz}`;
}

export function formatChunkCoordinate(coord: ChunkCoordinate): string {
  return formatChunkCoordinateKey(coord.x, coord.y, coord.z);
}

export function formatColumnCoordinateKey(cx: number, cz: number): string {
  return `${cx}:${cz}`;
}

export function formatRenderSummaryRegionCoordinateKey(coord: RenderSummaryRegionCoordinate): string {
  return `${coord.x}:${coord.z}`;
}

export function formatRenderSummaryRegionCoordinateParts(regionX: number, regionZ: number): string {
  return `${regionX}:${regionZ}`;
}
