import type { ChunkCoordinate } from "./types.ts";

export interface ChunkVoxelEdit {
  readonly x: number;
  readonly y: number;
  readonly z: number;
  readonly material: number;
}

export interface PackedChunkEditDelta {
  readonly revision: number;
  readonly voxelIndices: readonly number[];
  readonly materials: readonly number[];
}

export interface ChunkEditJournal {
  readonly coord: ChunkCoordinate;
  readonly chunkSize: number;
  readonly minRevision: number | null;
  readonly maxRevision: number | null;
  readonly deltas: readonly PackedChunkEditDelta[];
}

export interface ExportedChunkEditJournal {
  readonly version: 1;
  readonly coord: ChunkCoordinate;
  readonly chunkSize: number;
  readonly minRevision: number | null;
  readonly maxRevision: number | null;
  readonly deltas: readonly PackedChunkEditDelta[];
}

interface IndexedMaterialEdit {
  readonly voxelIndex: number;
  readonly material: number;
}

interface OrderedPackedChunkEditDelta extends PackedChunkEditDelta {
  readonly order: number;
}

export function packChunkEditDelta(
  revision: number,
  edits: Iterable<ChunkVoxelEdit>,
  chunkSize: number,
): PackedChunkEditDelta {
  assertRevision(revision);
  assertChunkSize(chunkSize);
  const latestByIndex = new Map<number, number>();
  for (const edit of edits) {
    const voxelIndex = toChunkVoxelIndex(edit, chunkSize);
    assertMaterial(edit.material);
    latestByIndex.set(voxelIndex, edit.material);
  }
  return packIndexedMaterialEdits(revision, latestByIndex);
}

export function createChunkEditJournal(
  coord: ChunkCoordinate,
  chunkSize: number,
  deltas: Iterable<PackedChunkEditDelta> = [],
): ChunkEditJournal {
  assertChunkSize(chunkSize);
  const orderedDeltas = normalizeDeltas(deltas, chunkSize);
  return {
    coord: cloneCoord(coord),
    chunkSize,
    minRevision: orderedDeltas[0]?.revision ?? null,
    maxRevision: orderedDeltas[orderedDeltas.length - 1]?.revision ?? null,
    deltas: orderedDeltas.map(stripDeltaOrder),
  };
}

export function replayChunkEditJournal(journal: ChunkEditJournal, baseData: Uint16Array): Uint16Array {
  assertChunkSize(journal.chunkSize);
  assertChunkData(baseData, journal.chunkSize);
  const replayed = new Uint16Array(baseData);
  for (const delta of normalizeDeltas(journal.deltas, journal.chunkSize)) {
    applyPackedChunkEditDeltaInPlace(replayed, delta, journal.chunkSize);
  }
  return replayed;
}

export function applyPackedChunkEditDelta(
  baseData: Uint16Array,
  delta: PackedChunkEditDelta,
  chunkSize: number,
): Uint16Array {
  assertChunkSize(chunkSize);
  assertChunkData(baseData, chunkSize);
  const replayed = new Uint16Array(baseData);
  applyPackedChunkEditDeltaInPlace(replayed, normalizeDelta(delta, chunkSize), chunkSize);
  return replayed;
}

export function compactChunkEditJournal(journal: ChunkEditJournal): ChunkEditJournal {
  assertChunkSize(journal.chunkSize);
  const winners = new Map<number, { material: number; revision: number; order: number }>();
  for (const delta of normalizeDeltas(journal.deltas, journal.chunkSize)) {
    for (let index = 0; index < delta.voxelIndices.length; index += 1) {
      const voxelIndex = delta.voxelIndices[index]!;
      winners.set(voxelIndex, {
        material: delta.materials[index]!,
        revision: delta.revision,
        order: delta.order,
      });
    }
  }

  const groupedByRevision = new Map<number, Map<number, number>>();
  for (const [voxelIndex, winner] of [...winners.entries()].sort(compareCompactedWinners)) {
    let edits = groupedByRevision.get(winner.revision);
    if (!edits) {
      edits = new Map<number, number>();
      groupedByRevision.set(winner.revision, edits);
    }
    edits.set(voxelIndex, winner.material);
  }

  const compactedDeltas = [...groupedByRevision.entries()]
    .sort(([leftRevision], [rightRevision]) => leftRevision - rightRevision)
    .map(([revision, edits]) => packIndexedMaterialEdits(revision, edits));
  return createChunkEditJournal(journal.coord, journal.chunkSize, compactedDeltas);
}

export function exportChunkEditJournal(journal: ChunkEditJournal): ExportedChunkEditJournal {
  const normalized = createChunkEditJournal(journal.coord, journal.chunkSize, journal.deltas);
  return {
    version: 1,
    coord: cloneCoord(normalized.coord),
    chunkSize: normalized.chunkSize,
    minRevision: normalized.minRevision,
    maxRevision: normalized.maxRevision,
    deltas: normalized.deltas.map((delta) => ({
      revision: delta.revision,
      voxelIndices: [...delta.voxelIndices],
      materials: [...delta.materials],
    })),
  };
}

export function importChunkEditJournal(value: ExportedChunkEditJournal): ChunkEditJournal {
  if (value.version !== 1) {
    throw new Error(`Unsupported chunk edit journal version ${value.version}`);
  }
  return createChunkEditJournal(value.coord, value.chunkSize, value.deltas);
}

export function toChunkVoxelIndex(
  voxel: Pick<ChunkVoxelEdit, "x" | "y" | "z">,
  chunkSize: number,
): number {
  assertChunkSize(chunkSize);
  assertLocalCoordinate("x", voxel.x, chunkSize);
  assertLocalCoordinate("y", voxel.y, chunkSize);
  assertLocalCoordinate("z", voxel.z, chunkSize);
  return voxel.x + voxel.y * chunkSize + voxel.z * chunkSize * chunkSize;
}

function applyPackedChunkEditDeltaInPlace(
  data: Uint16Array,
  delta: PackedChunkEditDelta,
  chunkSize: number,
): void {
  const normalized = normalizeDelta(delta, chunkSize);
  for (let index = 0; index < normalized.voxelIndices.length; index += 1) {
    data[normalized.voxelIndices[index]!] = normalized.materials[index]!;
  }
}

function normalizeDeltas(
  deltas: Iterable<PackedChunkEditDelta>,
  chunkSize: number,
): OrderedPackedChunkEditDelta[] {
  return [...deltas]
    .map((delta, order) => ({ ...normalizeDelta(delta, chunkSize), order }))
    .sort((left, right) => left.revision - right.revision || left.order - right.order);
}

function normalizeDelta(delta: PackedChunkEditDelta, chunkSize: number): PackedChunkEditDelta {
  assertRevision(delta.revision);
  if (delta.voxelIndices.length !== delta.materials.length) {
    throw new Error("Packed chunk edit delta index/material lengths must match");
  }
  const latestByIndex = new Map<number, number>();
  for (let index = 0; index < delta.voxelIndices.length; index += 1) {
    const voxelIndex = delta.voxelIndices[index]!;
    const material = delta.materials[index]!;
    assertVoxelIndex(voxelIndex, chunkSize);
    assertMaterial(material);
    latestByIndex.set(voxelIndex, material);
  }
  return packIndexedMaterialEdits(delta.revision, latestByIndex);
}

function stripDeltaOrder(delta: OrderedPackedChunkEditDelta): PackedChunkEditDelta {
  return {
    revision: delta.revision,
    voxelIndices: delta.voxelIndices,
    materials: delta.materials,
  };
}

function packIndexedMaterialEdits(
  revision: number,
  editsByIndex: ReadonlyMap<number, number>,
): PackedChunkEditDelta {
  const edits: IndexedMaterialEdit[] = [...editsByIndex.entries()]
    .map(([voxelIndex, material]) => ({ voxelIndex, material }))
    .sort((left, right) => left.voxelIndex - right.voxelIndex);
  return {
    revision,
    voxelIndices: edits.map((edit) => edit.voxelIndex),
    materials: edits.map((edit) => edit.material),
  };
}

function compareCompactedWinners(
  [leftIndex, left]: readonly [number, { readonly revision: number; readonly order: number }],
  [rightIndex, right]: readonly [number, { readonly revision: number; readonly order: number }],
): number {
  return left.revision - right.revision
    || left.order - right.order
    || leftIndex - rightIndex;
}

function assertChunkSize(chunkSize: number): void {
  if (!Number.isInteger(chunkSize) || chunkSize <= 0) {
    throw new Error(`Expected positive integer chunk size, received ${chunkSize}`);
  }
}

function assertChunkData(data: Uint16Array, chunkSize: number): void {
  const expectedLength = chunkSize * chunkSize * chunkSize;
  if (data.length !== expectedLength) {
    throw new Error(`Expected chunk data length ${expectedLength}, received ${data.length}`);
  }
}

function assertLocalCoordinate(axis: string, value: number, chunkSize: number): void {
  if (!Number.isInteger(value) || value < 0 || value >= chunkSize) {
    throw new Error(`Expected local ${axis} in [0, ${chunkSize}), received ${value}`);
  }
}

function assertVoxelIndex(voxelIndex: number, chunkSize: number): void {
  const maxIndexExclusive = chunkSize * chunkSize * chunkSize;
  if (!Number.isInteger(voxelIndex) || voxelIndex < 0 || voxelIndex >= maxIndexExclusive) {
    throw new Error(`Expected voxel index in [0, ${maxIndexExclusive}), received ${voxelIndex}`);
  }
}

function assertRevision(revision: number): void {
  if (!Number.isInteger(revision) || revision <= 0) {
    throw new Error(`Expected positive integer edit revision, received ${revision}`);
  }
}

function assertMaterial(material: number): void {
  if (!Number.isInteger(material) || material < 0 || material > 0xffff) {
    throw new Error(`Expected uint16 material, received ${material}`);
  }
}

function cloneCoord(coord: ChunkCoordinate): ChunkCoordinate {
  return { x: coord.x, y: coord.y, z: coord.z };
}
