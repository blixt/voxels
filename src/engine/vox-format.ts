import { VoxelWorld } from "./world.ts";

interface VoxModel {
  size: [number, number, number];
  voxels: Array<{ x: number; y: number; z: number; colorIndex: number }>;
}

interface VoxTransformNode {
  kind: "transform";
  id: number;
  childId: number;
  translation: [number, number, number];
}

interface VoxGroupNode {
  kind: "group";
  id: number;
  childIds: number[];
}

interface VoxShapeNode {
  kind: "shape";
  id: number;
  modelIds: number[];
}

type VoxNode = VoxTransformNode | VoxGroupNode | VoxShapeNode;

class Reader {
  offset = 0;

  constructor(private readonly bytes: Uint8Array) {}

  readI32(): number {
    if (this.offset + 4 > this.bytes.length) {
      throw new Error("Unexpected end of file");
    }
    const value =
      (this.bytes[this.offset]!) |
      (this.bytes[this.offset + 1]! << 8) |
      (this.bytes[this.offset + 2]! << 16) |
      (this.bytes[this.offset + 3]! << 24);
    this.offset += 4;
    return value | 0;
  }

  readU8(): number {
    const value = this.bytes[this.offset];
    if (value === undefined) {
      throw new Error("Unexpected end of file");
    }
    this.offset += 1;
    return value;
  }

  readString(length: number): string {
    const slice = this.bytes.subarray(this.offset, this.offset + length);
    this.offset += length;
    return new TextDecoder().decode(slice);
  }

  readDictionary(): Record<string, string> {
    const entryCount = this.readI32();
    const result: Record<string, string> = {};
    for (let index = 0; index < entryCount; index += 1) {
      const keyLength = this.readI32();
      const key = this.readString(keyLength);
      const valueLength = this.readI32();
      const value = this.readString(valueLength);
      result[key] = value;
    }
    return result;
  }

  skip(length: number): void {
    this.offset += length;
  }
}

export interface VoxImportResult {
  world: VoxelWorld;
  warnings: string[];
}

export function importMagicaVoxel(
  bytes: Uint8Array | ArrayBuffer,
  chunkSize = 32,
): VoxImportResult {
  const source = bytes instanceof Uint8Array ? bytes : new Uint8Array(bytes);
  const reader = new Reader(source);
  const magic = reader.readString(4);
  if (magic !== "VOX ") {
    throw new Error("Invalid VOX file");
  }
  const version = reader.readI32();
  if (version < 150) {
    throw new Error(`Unsupported VOX version: ${version}`);
  }

  const mainId = reader.readString(4);
  if (mainId !== "MAIN") {
    throw new Error("VOX file is missing MAIN chunk");
  }
  reader.readI32();
  const mainChildrenSize = reader.readI32();
  const mainEnd = reader.offset + mainChildrenSize;

  const models: VoxModel[] = [];
  const nodes = new Map<number, VoxNode>();
  const warnings: string[] = [];
  const palette = new Array<number>(256).fill(0);
  let pendingSize: [number, number, number] | null = null;

  while (reader.offset < mainEnd) {
    const id = reader.readString(4);
    const chunkContentSize = reader.readI32();
    const childSize = reader.readI32();
    const chunkStart = reader.offset;

    switch (id) {
      case "SIZE": {
        pendingSize = [reader.readI32(), reader.readI32(), reader.readI32()];
        break;
      }
      case "XYZI": {
        const voxelCount = reader.readI32();
        const size = pendingSize ?? [16, 16, 16];
        const voxels = new Array<{ x: number; y: number; z: number; colorIndex: number }>(voxelCount);
        for (let index = 0; index < voxelCount; index += 1) {
          voxels[index] = {
            x: reader.readU8(),
            y: reader.readU8(),
            z: reader.readU8(),
            colorIndex: reader.readU8(),
          };
        }
        models.push({ size, voxels });
        pendingSize = null;
        break;
      }
      case "RGBA": {
        for (let index = 1; index <= 255; index += 1) {
          const r = reader.readU8();
          const g = reader.readU8();
          const b = reader.readU8();
          const a = reader.readU8();
          palette[index] = ((a << 24) | (b << 16) | (g << 8) | r) >>> 0;
        }
        break;
      }
      case "nTRN": {
        const idValue = reader.readI32();
        reader.readDictionary();
        const childId = reader.readI32();
        reader.readI32();
        reader.readI32();
        const numFrames = reader.readI32();
        let translation: [number, number, number] = [0, 0, 0];
        for (let frameIndex = 0; frameIndex < numFrames; frameIndex += 1) {
          const frameAttributes = reader.readDictionary();
          if (frameAttributes._r) {
            warnings.push("VOX import ignored MagicaVoxel rotation metadata (_r)");
          }
          if (frameAttributes._t) {
            const [x, y, z] = frameAttributes._t.split(" ").map((value) => Number.parseInt(value, 10));
            translation = [x ?? 0, y ?? 0, z ?? 0];
          }
        }
        nodes.set(idValue, { kind: "transform", id: idValue, childId, translation });
        break;
      }
      case "nGRP": {
        const idValue = reader.readI32();
        reader.readDictionary();
        const childCount = reader.readI32();
        const childIds = new Array<number>(childCount);
        for (let index = 0; index < childCount; index += 1) {
          childIds[index] = reader.readI32();
        }
        nodes.set(idValue, { kind: "group", id: idValue, childIds });
        break;
      }
      case "nSHP": {
        const idValue = reader.readI32();
        reader.readDictionary();
        const modelCount = reader.readI32();
        const modelIds = new Array<number>(modelCount);
        for (let index = 0; index < modelCount; index += 1) {
          modelIds[index] = reader.readI32();
          reader.readDictionary();
        }
        nodes.set(idValue, { kind: "shape", id: idValue, modelIds });
        break;
      }
      default:
        reader.skip(chunkContentSize);
        break;
    }

    reader.offset = chunkStart + chunkContentSize;
    if (childSize > 0) {
      reader.skip(childSize);
    }
  }

  const usedModels = resolveModelPlacements(nodes, warnings);
  const placements = usedModels.length > 0
    ? usedModels
    : models.map((_, index) => ({
        modelIndex: index,
        translation: [0, 0, 0] as [number, number, number],
      }));

  const bounds = measurePlacements(models, placements);
  const world = new VoxelWorld(
    {
      width: Math.max(bounds.width, 1),
      height: Math.max(bounds.height, 1),
      depth: Math.max(bounds.depth, 1),
    },
    chunkSize,
    palette,
  );

  for (const placement of placements) {
    const model = models[placement.modelIndex];
    if (!model) {
      continue;
    }
    for (const voxel of model.voxels) {
      world.setVoxel(
        voxel.x + placement.translation[0],
        voxel.z + placement.translation[1],
        voxel.y + placement.translation[2],
        voxel.colorIndex,
      );
    }
  }

  return { world, warnings };
}

function resolveModelPlacements(
  nodes: Map<number, VoxNode>,
  warnings: string[],
): Array<{ modelIndex: number; translation: [number, number, number] }> {
  const root = nodes.get(0);
  if (!root) {
    return [];
  }
  const placements: Array<{ modelIndex: number; translation: [number, number, number] }> = [];

  const visit = (nodeId: number, translation: [number, number, number]): void => {
    const node = nodes.get(nodeId);
    if (!node) {
      warnings.push(`VOX import skipped missing scene graph node ${nodeId}`);
      return;
    }
    switch (node.kind) {
      case "transform":
        visit(
          node.childId,
          [
            translation[0] + node.translation[0],
            translation[1] + node.translation[1],
            translation[2] + node.translation[2],
          ],
        );
        return;
      case "group":
        for (const childId of node.childIds) {
          visit(childId, translation);
        }
        return;
      case "shape":
        for (const modelIndex of node.modelIds) {
          placements.push({ modelIndex, translation: [...translation] as [number, number, number] });
        }
        return;
    }
  };

  visit(0, [0, 0, 0]);
  return placements;
}

function measurePlacements(
  models: VoxModel[],
  placements: Array<{ modelIndex: number; translation: [number, number, number] }>,
): { width: number; height: number; depth: number } {
  let maxX = 0;
  let maxY = 0;
  let maxZ = 0;
  for (const placement of placements) {
    const model = models[placement.modelIndex];
    if (!model) {
      continue;
    }
    maxX = Math.max(maxX, placement.translation[0] + model.size[0]);
    maxY = Math.max(maxY, placement.translation[1] + model.size[2]);
    maxZ = Math.max(maxZ, placement.translation[2] + model.size[1]);
  }
  return { width: maxX, height: maxY, depth: maxZ };
}
