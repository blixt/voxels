import {
  buildFirstPersonCameraMatrices,
  type FirstPersonCameraState,
} from "./first-person-camera.ts";
import { dotVec3 } from "./math.ts";
import type { Vec3 } from "./types.ts";
import type { VoxelRayHit } from "./voxel-raycast.ts";

export type ScreenPoint = readonly [number, number];

export interface TargetingOverlaySegment {
  from: ScreenPoint;
  to: ScreenPoint;
}

export interface TargetingOverlayGeometry {
  visible: boolean;
  viewportWidth: number;
  viewportHeight: number;
  outlineSegments: TargetingOverlaySegment[];
  facePolygon: ScreenPoint[];
}

interface CubeFaceDefinition {
  normal: Vec3;
  cornerIndices: readonly [number, number, number, number];
  edgeIndices: readonly number[];
}

const CUBE_CORNERS: ReadonlyArray<readonly [number, number, number]> = [
  [0, 0, 0],
  [1, 0, 0],
  [0, 1, 0],
  [1, 1, 0],
  [0, 0, 1],
  [1, 0, 1],
  [0, 1, 1],
  [1, 1, 1],
];

const CUBE_EDGES: ReadonlyArray<readonly [number, number]> = [
  [0, 1],
  [1, 3],
  [3, 2],
  [2, 0],
  [4, 5],
  [5, 7],
  [7, 6],
  [6, 4],
  [0, 4],
  [1, 5],
  [2, 6],
  [3, 7],
];

const CUBE_FACES: ReadonlyArray<CubeFaceDefinition> = [
  {
    normal: [-1, 0, 0],
    cornerIndices: [0, 2, 6, 4],
    edgeIndices: [3, 10, 7, 8],
  },
  {
    normal: [1, 0, 0],
    cornerIndices: [1, 5, 7, 3],
    edgeIndices: [9, 5, 11, 1],
  },
  {
    normal: [0, -1, 0],
    cornerIndices: [0, 4, 5, 1],
    edgeIndices: [8, 4, 9, 0],
  },
  {
    normal: [0, 1, 0],
    cornerIndices: [2, 3, 7, 6],
    edgeIndices: [2, 11, 6, 10],
  },
  {
    normal: [0, 0, -1],
    cornerIndices: [0, 1, 3, 2],
    edgeIndices: [0, 1, 2, 3],
  },
  {
    normal: [0, 0, 1],
    cornerIndices: [4, 6, 7, 5],
    edgeIndices: [7, 6, 5, 4],
  },
];

export function buildTargetingOverlayGeometry(
  camera: FirstPersonCameraState,
  hit: VoxelRayHit,
  viewportWidth: number,
  viewportHeight: number,
): TargetingOverlayGeometry {
  if (viewportWidth <= 0 || viewportHeight <= 0) {
    return hiddenTargetingOverlayGeometry(viewportWidth, viewportHeight);
  }

  const matrices = buildFirstPersonCameraMatrices(camera, viewportWidth / viewportHeight);
  const worldCorners = CUBE_CORNERS.map(([offsetX, offsetY, offsetZ]) => {
    return [
      hit.voxel[0] + offsetX,
      hit.voxel[1] + offsetY,
      hit.voxel[2] + offsetZ,
    ] as Vec3;
  });
  const projectedCorners = worldCorners.map((point) =>
    projectScreenPoint(matrices.viewProjection, point, viewportWidth, viewportHeight)
  );
  if (projectedCorners.some((point) => point === null)) {
    return hiddenTargetingOverlayGeometry(viewportWidth, viewportHeight);
  }

  const visibleEdges = new Set<number>();
  for (const face of CUBE_FACES) {
    const faceCenter = averageFaceCenter(worldCorners, face.cornerIndices);
    const toCamera: Vec3 = [
      camera.position[0] - faceCenter[0],
      camera.position[1] - faceCenter[1],
      camera.position[2] - faceCenter[2],
    ];
    if (dotVec3(face.normal, toCamera) <= 0) {
      continue;
    }
    for (const edgeIndex of face.edgeIndices) {
      visibleEdges.add(edgeIndex);
    }
  }

  const outlineSegments = Array.from(visibleEdges, (edgeIndex) => {
    const [startIndex, endIndex] = CUBE_EDGES[edgeIndex]!;
    return {
      from: projectedCorners[startIndex]!,
      to: projectedCorners[endIndex]!,
    };
  });

  const hitFace = CUBE_FACES.find((face) => sameNormal(face.normal, hit.normal));
  if (!hitFace) {
    return hiddenTargetingOverlayGeometry(viewportWidth, viewportHeight);
  }

  return {
    visible: outlineSegments.length > 0,
    viewportWidth,
    viewportHeight,
    outlineSegments,
    facePolygon: hitFace.cornerIndices.map((cornerIndex) => projectedCorners[cornerIndex]!),
  };
}

function hiddenTargetingOverlayGeometry(
  viewportWidth: number,
  viewportHeight: number,
): TargetingOverlayGeometry {
  return {
    visible: false,
    viewportWidth,
    viewportHeight,
    outlineSegments: [],
    facePolygon: [],
  };
}

function averageFaceCenter(
  corners: readonly Vec3[],
  cornerIndices: readonly [number, number, number, number],
): Vec3 {
  let x = 0;
  let y = 0;
  let z = 0;
  for (const cornerIndex of cornerIndices) {
    const corner = corners[cornerIndex]!;
    x += corner[0];
    y += corner[1];
    z += corner[2];
  }
  return [x * 0.25, y * 0.25, z * 0.25];
}

function sameNormal(a: Vec3, b: Vec3): boolean {
  return a[0] === b[0] && a[1] === b[1] && a[2] === b[2];
}

function projectScreenPoint(
  viewProjection: Float32Array,
  point: Vec3,
  viewportWidth: number,
  viewportHeight: number,
): ScreenPoint | null {
  const clipX = viewProjection[0]! * point[0]
    + viewProjection[4]! * point[1]
    + viewProjection[8]! * point[2]
    + viewProjection[12]!;
  const clipY = viewProjection[1]! * point[0]
    + viewProjection[5]! * point[1]
    + viewProjection[9]! * point[2]
    + viewProjection[13]!;
  const clipW = viewProjection[3]! * point[0]
    + viewProjection[7]! * point[1]
    + viewProjection[11]! * point[2]
    + viewProjection[15]!;
  if (clipW <= 0) {
    return null;
  }
  const ndcX = clipX / clipW;
  const ndcY = clipY / clipW;
  return [
    (ndcX * 0.5 + 0.5) * viewportWidth,
    (1 - (ndcY * 0.5 + 0.5)) * viewportHeight,
  ];
}
