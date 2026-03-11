import type { Vec3 } from "./types.ts";

export interface StreamAnchor {
  chunkX: number;
  chunkZ: number;
}

export function resolveStreamAnchor(
  current: StreamAnchor | null,
  playerChunkX: number,
  playerChunkZ: number,
  marginChunks: number,
): {
  anchor: StreamAnchor;
  changed: boolean;
} {
  const normalizedMargin = Math.max(0, Math.floor(marginChunks));
  if (!current) {
    return {
      anchor: { chunkX: playerChunkX, chunkZ: playerChunkZ },
      changed: true,
    };
  }
  if (
    Math.abs(playerChunkX - current.chunkX) <= normalizedMargin
    && Math.abs(playerChunkZ - current.chunkZ) <= normalizedMargin
  ) {
    return {
      anchor: current,
      changed: false,
    };
  }
  return {
    anchor: { chunkX: playerChunkX, chunkZ: playerChunkZ },
    changed: true,
  };
}

export function buildStreamAnchorPosition(anchor: StreamAnchor, chunkSize: number, y: number): Vec3 {
  return [
    anchor.chunkX * chunkSize + chunkSize * 0.5,
    y,
    anchor.chunkZ * chunkSize + chunkSize * 0.5,
  ];
}
