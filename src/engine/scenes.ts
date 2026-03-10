import { clamp, degToRad, hashUint32, packRgba } from "./math.ts";
import type { SceneBuildResult, SceneKind } from "./types.ts";
import { VoxelWorld } from "./world.ts";

const DEFAULT_WORLD_SIZE = 256;

function smoothstep(value: number): number {
  return value * value * (3 - 2 * value);
}

function lerp(a: number, b: number, t: number): number {
  return a + (b - a) * t;
}

function noise2D(x: number, z: number, seed: number): number {
  const x0 = Math.floor(x);
  const z0 = Math.floor(z);
  const tx = x - x0;
  const tz = z - z0;

  const h00 = hashUint32(x0 * 374761393 + z0 * 668265263 + seed) / 0xffffffff;
  const h10 = hashUint32((x0 + 1) * 374761393 + z0 * 668265263 + seed) / 0xffffffff;
  const h01 = hashUint32(x0 * 374761393 + (z0 + 1) * 668265263 + seed) / 0xffffffff;
  const h11 = hashUint32((x0 + 1) * 374761393 + (z0 + 1) * 668265263 + seed) / 0xffffffff;

  const sx = smoothstep(tx);
  const sz = smoothstep(tz);
  const nx0 = lerp(h00, h10, sx);
  const nx1 = lerp(h01, h11, sx);
  return lerp(nx0, nx1, sz);
}

function fbm(x: number, z: number, octaves: number, seed: number): number {
  let amplitude = 1;
  let frequency = 1;
  let total = 0;
  let sum = 0;
  for (let octave = 0; octave < octaves; octave += 1) {
    total += noise2D(x * frequency, z * frequency, seed + octave * 977) * amplitude;
    sum += amplitude;
    amplitude *= 0.5;
    frequency *= 2;
  }
  return total / sum;
}

export interface SceneDefinition {
  id: string;
  label: string;
  kind: SceneKind;
  stress: boolean;
  describe(): string;
  build(): SceneBuildResult;
}

function createCameraPreset(target: [number, number, number], zoom: number, distance: number): SceneBuildResult["camera"] {
  return {
    target,
    yaw: degToRad(45),
    pitch: degToRad(-35.264),
    distance,
    zoom,
  };
}

function createBasePalette(): number[] {
  return [
    0,
    packRgba(28, 41, 61),
    packRgba(86, 126, 62),
    packRgba(112, 86, 56),
    packRgba(136, 120, 96),
    packRgba(193, 185, 161),
    packRgba(89, 158, 206),
    packRgba(168, 134, 92),
    packRgba(176, 72, 60),
    packRgba(224, 214, 121),
    packRgba(108, 154, 98),
    packRgba(230, 236, 242),
  ];
}

function stampTree(world: VoxelWorld, x: number, y: number, z: number, trunk: number, leaves: number): void {
  world.fillBox(x, y, z, x + 1, y + 5, z + 1, trunk);
  world.fillBox(x - 2, y + 3, z - 2, x + 3, y + 6, z + 3, leaves);
  world.fillBox(x - 1, y + 6, z - 1, x + 2, y + 8, z + 2, leaves);
}

function stampHouse(world: VoxelWorld, x: number, y: number, z: number, wall: number, roof: number, trim: number): void {
  world.fillBox(x, y, z, x + 10, y + 1, z + 8, trim);
  world.fillBox(x, y + 1, z, x + 10, y + 6, z + 1, wall);
  world.fillBox(x, y + 1, z + 7, x + 10, y + 6, z + 8, wall);
  world.fillBox(x, y + 1, z + 1, x + 1, y + 6, z + 7, wall);
  world.fillBox(x + 9, y + 1, z + 1, x + 10, y + 6, z + 7, wall);
  world.fillBox(x + 4, y + 1, z, x + 6, y + 4, z + 1, 0);
  for (let level = 0; level < 4; level += 1) {
    world.fillBox(x - level, y + 6 + level, z - level, x + 10 + level, y + 7 + level, z + 8 + level, roof);
  }
}

export function createDefaultScene(): SceneBuildResult {
  const world = new VoxelWorld(
    { width: DEFAULT_WORLD_SIZE, height: DEFAULT_WORLD_SIZE, depth: DEFAULT_WORLD_SIZE },
    32,
    createBasePalette(),
  );

  const deepStone = 1;
  const grass = 2;
  const dirt = 3;
  const stone = 4;
  const sand = 5;
  const water = 6;
  const wood = 7;
  const brick = 8;
  const accent = 9;
  const leaves = 10;
  const plaster = 11;

  for (let z = 0; z < world.depth; z += 1) {
    for (let x = 0; x < world.width; x += 1) {
      const hill = fbm(x / 28, z / 28, 4, 1337);
      const ridge = fbm(x / 11, z / 11, 2, 7331);
      const plateau = fbm(x / 64, z / 64, 3, 9559);
      const height = Math.floor(28 + hill * 26 + ridge * 8 + plateau * 12);
      const shoreline = 34;
      for (let y = 0; y <= height; y += 1) {
        if (y < height - 10) {
          world.setVoxel(x, y, z, deepStone);
        } else if (y < height - 3) {
          world.setVoxel(x, y, z, stone);
        } else if (height < shoreline) {
          world.setVoxel(x, y, z, sand);
        } else if (y === height) {
          world.setVoxel(x, y, z, grass);
        } else {
          world.setVoxel(x, y, z, dirt);
        }
      }
      if (height < shoreline) {
        for (let y = height + 1; y <= shoreline; y += 1) {
          world.setVoxel(x, y, z, water);
        }
      }
    }
  }

  for (let index = 0; index < 28; index += 1) {
    const x = 24 + ((index * 29) % 180);
    const z = 40 + ((index * 47) % 160);
    const y = findSurface(world, x, z) + 1;
    stampTree(world, x, y, z, wood, leaves);
  }

  const housePositions: Array<[number, number]> = [
    [68, 88],
    [110, 122],
    [142, 84],
  ];
  for (const [x, z] of housePositions) {
    const y = findSurface(world, x + 4, z + 4) + 1;
    stampHouse(world, x, y, z, plaster, brick, wood);
  }

  world.fillBox(154, 47, 106, 168, 48, 120, accent);
  world.fillBox(160, 48, 112, 162, 56, 114, accent);

  return {
    name: "terrain256",
    world,
    notes: ["256^3 world", "Procedural terrain", "Isometric camera target"],
    camera: createCameraPreset([128, 52, 128], 90, 420),
  };
}

function createScatterScene(): SceneBuildResult {
  const world = new VoxelWorld(
    { width: DEFAULT_WORLD_SIZE, height: DEFAULT_WORLD_SIZE, depth: DEFAULT_WORLD_SIZE },
    32,
    createBasePalette(),
  );
  const accentColors = [2, 8, 9, 10, 11];
  for (let index = 0; index < 14000; index += 1) {
    const x = hashUint32(index * 17 + 1) % world.width;
    const y = 8 + (hashUint32(index * 31 + 7) % 96);
    const z = hashUint32(index * 43 + 13) % world.depth;
    const material = accentColors[index % accentColors.length]!;
    world.setVoxel(x, y, z, material);
  }
  return {
    name: "scatter256",
    world,
    notes: ["Sparse isolated voxels", "Draw-call and geometry stress"],
    camera: createCameraPreset([128, 56, 128], 92, 420),
  };
}

function createDenseCoreScene(): SceneBuildResult {
  const world = new VoxelWorld(
    { width: DEFAULT_WORLD_SIZE, height: DEFAULT_WORLD_SIZE, depth: DEFAULT_WORLD_SIZE },
    32,
    createBasePalette(),
  );
  const center = DEFAULT_WORLD_SIZE / 2;
  const radius = 64;
  for (let z = center - radius; z < center + radius; z += 1) {
    for (let y = center - radius; y < center + radius; y += 1) {
      for (let x = center - radius; x < center + radius; x += 1) {
        const dx = x - center;
        const dy = y - center;
        const dz = z - center;
        const distance = Math.sqrt(dx * dx + dy * dy + dz * dz);
        if (distance <= radius) {
          const blend = clamp(distance / radius, 0, 1);
          const material = blend > 0.8 ? 9 : blend > 0.55 ? 8 : 4;
          world.setVoxel(x, y, z, material);
        }
      }
    }
  }
  return {
    name: "denseCore128",
    world,
    notes: ["Large dense sphere", "Surface extraction stress"],
    camera: createCameraPreset([128, 128, 128], 88, 420),
  };
}

function createEditStormScene(): SceneBuildResult {
  const { world } = createDefaultScene();
  for (let index = 0; index < 1200; index += 1) {
    const x = 32 + (index * 11) % 180;
    const z = 32 + (index * 23) % 180;
    const y = findSurface(world, x, z) + 1 + (index % 6);
    world.setVoxel(x, y, z, (index % 2 === 0 ? 8 : 9));
  }
  return {
    name: "editStorm256",
    world,
    notes: ["Terrain with synthetic edit workload"],
    camera: createCameraPreset([128, 52, 128], 90, 420),
  };
}

function createStressDrawCallsScene(): SceneBuildResult {
  const world = new VoxelWorld(
    { width: DEFAULT_WORLD_SIZE, height: DEFAULT_WORLD_SIZE, depth: DEFAULT_WORLD_SIZE },
    32,
    createBasePalette(),
  );
  const inset = Math.floor(world.chunkSize * 0.5) - 1;
  const size = 2;
  for (let cz = 0; cz < world.chunkCountZ; cz += 1) {
    for (let cy = 0; cy < world.chunkCountY; cy += 1) {
      for (let cx = 0; cx < world.chunkCountX; cx += 1) {
        const material = 2 + ((cx + cy * 2 + cz * 3) % 9);
        const x = cx * world.chunkSize + inset;
        const y = cy * world.chunkSize + inset;
        const z = cz * world.chunkSize + inset;
        world.fillBox(x, y, z, x + size, y + size, z + size, material);
      }
    }
  }
  return {
    name: "stressDrawCalls512",
    world,
    notes: ["Stress scene", "One micro-cube per chunk", "Max active chunk draw calls"],
    camera: createCameraPreset([128, 128, 128], 118, 470),
  };
}

function createStressMicroCubesScene(): SceneBuildResult {
  const world = new VoxelWorld(
    { width: DEFAULT_WORLD_SIZE, height: DEFAULT_WORLD_SIZE, depth: DEFAULT_WORLD_SIZE },
    32,
    createBasePalette(),
  );
  const materials = [2, 5, 8, 9, 10, 11];
  for (let z = 24; z < 232; z += 8) {
    for (let y = 24; y < 216; y += 8) {
      for (let x = 24; x < 232; x += 8) {
        const material = materials[hashUint32(x * 17 + y * 31 + z * 43) % materials.length]!;
        world.fillBox(x, y, z, x + 2, y + 2, z + 2, material);
      }
    }
  }
  return {
    name: "stressMicroCubes256",
    world,
    notes: ["Stress scene", "Many isolated 2x2x2 cubes", "Poor greedy-merge case"],
    camera: createCameraPreset([128, 116, 128], 122, 500),
  };
}

function createStressScreensScene(): SceneBuildResult {
  const world = new VoxelWorld(
    { width: DEFAULT_WORLD_SIZE, height: DEFAULT_WORLD_SIZE, depth: DEFAULT_WORLD_SIZE },
    32,
    createBasePalette(),
  );
  const materials = [4, 6, 8, 9, 11];
  for (let screen = 0; screen < 12; screen += 1) {
    const material = materials[screen % materials.length]!;
    const x = 24 + screen * 16;
    for (let y = 12 + (screen % 3) * 2; y < 196; y += 16) {
      const yEnd = Math.min(y + 5, 196);
      for (let z = 20 + ((screen + Math.floor(y / 16)) % 2) * 10; z < 236; z += 28) {
        world.fillBox(x, y, z, x + 1, yEnd, Math.min(z + 18, 236), material);
      }
    }
  }
  return {
    name: "stressScreens256",
    world,
    notes: ["Stress scene", "Layered perforated screens", "Depth-overdraw and fill pressure"],
    camera: createCameraPreset([128, 96, 128], 108, 455),
  };
}

function createValidationBlocksScene(): SceneBuildResult {
  const world = new VoxelWorld({ width: 40, height: 32, depth: 40 }, 16, createBasePalette());
  world.fillBox(4, 0, 4, 30, 1, 30, 5);
  world.fillBox(6, 1, 6, 12, 7, 12, 8);
  world.fillBox(14, 1, 8, 20, 11, 14, 9);
  world.fillBox(22, 1, 6, 28, 5, 12, 2);
  world.fillBox(10, 1, 18, 18, 3, 26, 6);
  world.fillBox(20, 1, 20, 26, 9, 26, 11);
  world.fillBox(8, 7, 8, 10, 13, 10, 10);
  world.fillBox(24, 9, 22, 28, 10, 26, 7);
  return {
    name: "validationBlocks",
    world,
    notes: ["Validation scene", "Multiple heights", "Distinct colors"],
    camera: createCameraPreset([18, 6, 18], 22, 100),
  };
}

function createValidationSingleVoxelScene(): SceneBuildResult {
  const world = new VoxelWorld({ width: 12, height: 12, depth: 12 }, 12, createBasePalette());
  world.setVoxel(5, 1, 5, 8);
  return {
    name: "validationSingleVoxel",
    world,
    notes: ["Validation scene", "Single voxel", "Primitive placement"],
    camera: createCameraPreset([5.5, 1.5, 5.5], 5, 28),
  };
}

function createValidationCubeScene(): SceneBuildResult {
  const world = new VoxelWorld({ width: 12, height: 12, depth: 12 }, 12, createBasePalette());
  world.fillBox(4, 1, 4, 6, 3, 6, 9);
  return {
    name: "validationCube",
    world,
    notes: ["Validation scene", "2x2x2 cube", "Primitive silhouette"],
    camera: createCameraPreset([5, 2, 5], 6, 28),
  };
}

function createValidationBridgeScene(): SceneBuildResult {
  const world = new VoxelWorld({ width: 48, height: 32, depth: 48 }, 16, createBasePalette());
  world.fillBox(4, 0, 4, 40, 1, 40, 4);
  world.fillBox(10, 1, 10, 14, 10, 14, 8);
  world.fillBox(26, 1, 10, 30, 10, 14, 8);
  world.fillBox(10, 9, 10, 30, 11, 14, 11);
  world.fillBox(16, 1, 22, 20, 6, 26, 2);
  world.fillBox(22, 1, 22, 26, 14, 26, 9);
  world.fillBox(28, 1, 22, 34, 4, 28, 10);
  world.fillBox(12, 1, 30, 18, 2, 36, 6);
  world.fillBox(20, 1, 30, 24, 2, 34, 5);
  world.fillBox(20, 2, 30, 24, 3, 34, 0);
  return {
    name: "validationBridge",
    world,
    notes: ["Validation scene", "Occlusion", "Floating bridge"],
    camera: createCameraPreset([22, 7, 22], 24, 108),
  };
}

export function findSurface(world: VoxelWorld, x: number, z: number): number {
  for (let y = world.height - 1; y >= 0; y -= 1) {
    if (world.getVoxel(x, y, z) !== 0) {
      return y;
    }
  }
  return 0;
}

export function getSceneDefinitions(): SceneDefinition[] {
  return [
    {
      id: "terrain256",
      label: "Terrain 256",
      kind: "performance",
      stress: false,
      describe: () => "Procedural settlement scene inside a 256x256x256 world.",
      build: createDefaultScene,
    },
    {
      id: "scatter256",
      label: "Scatter 256",
      kind: "performance",
      stress: false,
      describe: () => "Many isolated voxels distributed through the full volume.",
      build: createScatterScene,
    },
    {
      id: "denseCore128",
      label: "Dense Core",
      kind: "performance",
      stress: false,
      describe: () => "A large solid sphere to stress meshing on dense volumes.",
      build: createDenseCoreScene,
    },
    {
      id: "editStorm256",
      label: "Edit Storm",
      kind: "performance",
      stress: true,
      describe: () => "Terrain with a large batch of live-edit style modifications.",
      build: createEditStormScene,
    },
    {
      id: "stressDrawCalls512",
      label: "Stress: Draw Calls",
      kind: "performance",
      stress: true,
      describe: () => "Maximizes active chunk draw calls with a micro-cube in every chunk.",
      build: createStressDrawCallsScene,
    },
    {
      id: "stressMicroCubes256",
      label: "Stress: Micro Cubes",
      kind: "performance",
      stress: true,
      describe: () => "Fills the view with isolated 2x2x2 cubes to stress tiny-surface throughput.",
      build: createStressMicroCubesScene,
    },
    {
      id: "stressScreens256",
      label: "Stress: Screens",
      kind: "performance",
      stress: true,
      describe: () => "Stacks perforated screens to stress depth overdraw and fill-heavy views.",
      build: createStressScreensScene,
    },
    {
      id: "validationSingleVoxel",
      label: "Validation: Single Voxel",
      kind: "validation",
      stress: false,
      describe: () => "Tiny primitive check for voxel placement, framing, and shading.",
      build: createValidationSingleVoxelScene,
    },
    {
      id: "validationCube",
      label: "Validation: Cube",
      kind: "validation",
      stress: false,
      describe: () => "Tiny primitive check for a 2x2x2 cube silhouette and face visibility.",
      build: createValidationCubeScene,
    },
    {
      id: "validationBlocks",
      label: "Validation: Blocks",
      kind: "validation",
      stress: false,
      describe: () => "Compact validation scene with clear silhouette, colors, and depth ordering.",
      build: createValidationBlocksScene,
    },
    {
      id: "validationBridge",
      label: "Validation: Bridge",
      kind: "validation",
      stress: false,
      describe: () => "Compact validation scene with occlusion and floating geometry checks.",
      build: createValidationBridgeScene,
    },
  ];
}

export function getStressSceneDefinitions(): SceneDefinition[] {
  return getSceneDefinitions().filter((definition) => definition.stress);
}
