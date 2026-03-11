import type { ResidentChunkWorld } from "./world.ts";
import type { Vec3 } from "./types.ts";
import { metersToWorldUnits } from "./scale.ts";

export const PLAYER_RADIUS = metersToWorldUnits(0.3);
export const PLAYER_HEIGHT = metersToWorldUnits(1.8);
export const PLAYER_EYE_HEIGHT = metersToWorldUnits(1.68);
export const PLAYER_GRAVITY = metersToWorldUnits(10);
export const PLAYER_JUMP_VELOCITY = metersToWorldUnits(4.7);
export const PLAYER_BASE_MOVE_SPEED = metersToWorldUnits(4.6);
export const PLAYER_FAST_MOVE_MULTIPLIER = 1.57;
export const PLAYER_SLOW_MOVE_MULTIPLIER = 0.4;
export const PLAYER_MAX_STEP_HEIGHT = metersToWorldUnits(0.3);

const SKIN_WIDTH = 0.001;

export interface PlayerBodyState {
  feetPosition: Vec3;
  velocity: Vec3;
  radius: number;
  height: number;
  eyeHeight: number;
  grounded: boolean;
  bodyInWater: boolean;
  eyeInWater: boolean;
}

export interface PlayerPhysicsCommand {
  wishVelocity: Vec3;
  jump: boolean;
}

export interface PlayerStepInput {
  forward: number;
  strafe: number;
  jump: boolean;
  sprint: boolean;
  precision: boolean;
}

export interface PlayerStepResult {
  moved: boolean;
  grounded: boolean;
  collidedX: boolean;
  collidedY: boolean;
  collidedZ: boolean;
}

export type PlayerState = PlayerBodyState;

export function createPlayerBody(
  feetPosition: Vec3,
  options: Partial<Pick<PlayerBodyState, "radius" | "height" | "eyeHeight" | "grounded">> = {},
): PlayerBodyState {
  return {
    feetPosition: [...feetPosition],
    velocity: [0, 0, 0],
    radius: options.radius ?? PLAYER_RADIUS,
    height: options.height ?? PLAYER_HEIGHT,
    eyeHeight: options.eyeHeight ?? PLAYER_EYE_HEIGHT,
    grounded: options.grounded ?? false,
    bodyInWater: false,
    eyeInWater: false,
  };
}

export function createPlayerState(
  feetPosition: Vec3,
  options: Partial<Pick<PlayerBodyState, "radius" | "height" | "eyeHeight" | "grounded">> = {},
): PlayerState {
  return createPlayerBody(feetPosition, options);
}

export function getPlayerEyePosition(body: PlayerBodyState): Vec3 {
  return [
    body.feetPosition[0],
    body.feetPosition[1] + body.eyeHeight,
    body.feetPosition[2],
  ];
}

export function setPlayerEyePosition(body: PlayerBodyState, eyePosition: Vec3): void {
  body.feetPosition = [
    eyePosition[0],
    eyePosition[1] - body.eyeHeight,
    eyePosition[2],
  ];
  body.velocity = [0, 0, 0];
  body.grounded = false;
  body.bodyInWater = false;
  body.eyeInWater = false;
}

export function teleportPlayerToEyePosition(body: PlayerState, eyePosition: Vec3): void {
  setPlayerEyePosition(body, eyePosition);
}

export function stepPlayerBody(
  world: Pick<ResidentChunkWorld, "getVoxel" | "isCollisionMaterial" | "isWaterMaterial">,
  body: PlayerBodyState,
  command: PlayerPhysicsCommand,
  deltaSeconds: number,
): {
  collidedX: boolean;
  collidedY: boolean;
  collidedZ: boolean;
} {
  body.velocity[0] = command.wishVelocity[0];
  body.velocity[2] = command.wishVelocity[2];

  if (body.grounded && command.jump) {
    body.velocity[1] = PLAYER_JUMP_VELOCITY;
    body.grounded = false;
  }
  body.velocity[1] -= PLAYER_GRAVITY * deltaSeconds;

  const collidedY = moveAlongAxis(world, body, 1, body.velocity[1] * deltaSeconds);
  if (collidedY && body.velocity[1] <= 0) {
    body.grounded = true;
    body.velocity[1] = 0;
  } else if (!collidedY) {
    body.grounded = false;
  }
  const collidedX = moveAlongAxis(world, body, 0, body.velocity[0] * deltaSeconds);
  const steppedX = collidedX && body.grounded
    ? tryStepUp(world, body, 0, body.velocity[0] * deltaSeconds)
    : false;
  const collidedZ = moveAlongAxis(world, body, 2, body.velocity[2] * deltaSeconds);
  const steppedZ = collidedZ && body.grounded
    ? tryStepUp(world, body, 2, body.velocity[2] * deltaSeconds)
    : false;
  updateWaterState(world, body);
  return {
    collidedX: collidedX && !steppedX,
    collidedY,
    collidedZ: collidedZ && !steppedZ,
  };
}

export function stepPlayer(
  world: Pick<ResidentChunkWorld, "getVoxel" | "isCollisionMaterial" | "isWaterMaterial">,
  body: PlayerState,
  yaw: number,
  input: PlayerStepInput,
  deltaSeconds: number,
): PlayerStepResult {
  const before: Vec3 = [...body.feetPosition];
  const collisions = stepPlayerBody(world, body, {
    wishVelocity: buildWishVelocity(yaw, input),
    jump: input.jump,
  }, deltaSeconds);
  return {
    moved: body.feetPosition[0] !== before[0]
      || body.feetPosition[1] !== before[1]
      || body.feetPosition[2] !== before[2],
    grounded: body.grounded,
    collidedX: collisions.collidedX,
    collidedY: collisions.collidedY,
    collidedZ: collisions.collidedZ,
  };
}

function moveAlongAxis(
  world: Pick<ResidentChunkWorld, "getVoxel" | "isCollisionMaterial">,
  body: PlayerBodyState,
  axis: 0 | 1 | 2,
  delta: number,
): boolean {
  if (delta === 0) {
    return false;
  }
  const nextPosition: Vec3 = [...body.feetPosition];
  nextPosition[axis] += delta;
  const currentBounds = getPlayerBounds(body.feetPosition, body.radius, body.height);
  const nextBounds = getPlayerBounds(nextPosition, body.radius, body.height);
  const minX = Math.floor(Math.min(currentBounds.min[0], nextBounds.min[0]) + SKIN_WIDTH);
  const maxX = Math.floor(Math.max(currentBounds.max[0], nextBounds.max[0]) - SKIN_WIDTH);
  const minY = Math.floor(Math.min(currentBounds.min[1], nextBounds.min[1]) + SKIN_WIDTH);
  const maxY = Math.floor(Math.max(currentBounds.max[1], nextBounds.max[1]) - SKIN_WIDTH);
  const minZ = Math.floor(Math.min(currentBounds.min[2], nextBounds.min[2]) + SKIN_WIDTH);
  const maxZ = Math.floor(Math.max(currentBounds.max[2], nextBounds.max[2]) - SKIN_WIDTH);

  let resolved = nextPosition[axis];
  let collided = false;
  for (let z = minZ; z <= maxZ; z += 1) {
    for (let y = minY; y <= maxY; y += 1) {
      for (let x = minX; x <= maxX; x += 1) {
        const material = world.getVoxel(x, y, z);
        if (!world.isCollisionMaterial(material)) {
          continue;
        }
        collided = true;
        if (axis === 0) {
          resolved = delta > 0 ? Math.min(resolved, x - body.radius) : Math.max(resolved, x + 1 + body.radius);
        } else if (axis === 1) {
          resolved = delta > 0 ? Math.min(resolved, y - body.height) : Math.max(resolved, y + 1);
        } else {
          resolved = delta > 0 ? Math.min(resolved, z - body.radius) : Math.max(resolved, z + 1 + body.radius);
        }
      }
    }
  }
  body.feetPosition[axis] = resolved;
  return collided;
}

function tryStepUp(
  world: Pick<ResidentChunkWorld, "getVoxel" | "isCollisionMaterial">,
  body: PlayerBodyState,
  axis: 0 | 2,
  delta: number,
): boolean {
  if (delta === 0) {
    return false;
  }
  const originalPosition: Vec3 = [...body.feetPosition];
  for (let stepHeight = 1; stepHeight <= PLAYER_MAX_STEP_HEIGHT; stepHeight += 1) {
    const steppedBody: PlayerBodyState = {
      ...body,
      feetPosition: [
        originalPosition[0],
        originalPosition[1] + stepHeight,
        originalPosition[2],
      ],
      velocity: [...body.velocity],
    };
    if (collidesAt(world, steppedBody.feetPosition, steppedBody.radius, steppedBody.height)) {
      continue;
    }
    if (moveAlongAxis(world, steppedBody, axis, delta)) {
      continue;
    }
    if (!settleStepDown(world, steppedBody, stepHeight)) {
      continue;
    }
    body.feetPosition = steppedBody.feetPosition;
    body.grounded = true;
    return true;
  }
  return false;
}

function settleStepDown(
  world: Pick<ResidentChunkWorld, "getVoxel" | "isCollisionMaterial">,
  body: PlayerBodyState,
  maxDrop: number,
): boolean {
  const startY = body.feetPosition[1];
  for (let drop = 1; drop <= maxDrop; drop += 1) {
    const loweredPosition: Vec3 = [body.feetPosition[0], startY - drop, body.feetPosition[2]];
    if (!collidesAt(world, loweredPosition, body.radius, body.height)) {
      continue;
    }
    body.feetPosition[1] = loweredPosition[1] + 1;
    return true;
  }
  return false;
}

function collidesAt(
  world: Pick<ResidentChunkWorld, "getVoxel" | "isCollisionMaterial">,
  feetPosition: Vec3,
  radius: number,
  height: number,
): boolean {
  const bounds = getPlayerBounds(feetPosition, radius, height);
  const minX = Math.floor(bounds.min[0] + SKIN_WIDTH);
  const maxX = Math.floor(bounds.max[0] - SKIN_WIDTH);
  const minY = Math.floor(bounds.min[1] + SKIN_WIDTH);
  const maxY = Math.floor(bounds.max[1] - SKIN_WIDTH);
  const minZ = Math.floor(bounds.min[2] + SKIN_WIDTH);
  const maxZ = Math.floor(bounds.max[2] - SKIN_WIDTH);
  for (let z = minZ; z <= maxZ; z += 1) {
    for (let y = minY; y <= maxY; y += 1) {
      for (let x = minX; x <= maxX; x += 1) {
        if (world.isCollisionMaterial(world.getVoxel(x, y, z))) {
          return true;
        }
      }
    }
  }
  return false;
}

function getPlayerBounds(
  feetPosition: Vec3,
  radius: number,
  height: number,
): {
  min: Vec3;
  max: Vec3;
} {
  return {
    min: [
      feetPosition[0] - radius,
      feetPosition[1],
      feetPosition[2] - radius,
    ],
    max: [
      feetPosition[0] + radius,
      feetPosition[1] + height,
      feetPosition[2] + radius,
    ],
  };
}

function updateWaterState(
  world: Pick<ResidentChunkWorld, "getVoxel" | "isWaterMaterial">,
  body: PlayerBodyState,
): void {
  const bodySampleY = body.feetPosition[1] + Math.max(1, body.height * 0.35);
  const eyePosition = getPlayerEyePosition(body);
  body.bodyInWater = isWaterAt(world, body.feetPosition[0], bodySampleY, body.feetPosition[2]);
  body.eyeInWater = isWaterAt(world, eyePosition[0], eyePosition[1], eyePosition[2]);
}

function isWaterAt(
  world: Pick<ResidentChunkWorld, "getVoxel" | "isWaterMaterial">,
  x: number,
  y: number,
  z: number,
): boolean {
  const material = world.getVoxel(Math.floor(x), Math.floor(y), Math.floor(z));
  return world.isWaterMaterial(material);
}

function buildWishVelocity(yaw: number, input: PlayerStepInput): Vec3 {
  if (input.forward === 0 && input.strafe === 0) {
    return [0, 0, 0];
  }
  let speed = PLAYER_BASE_MOVE_SPEED;
  if (input.sprint) {
    speed *= PLAYER_FAST_MOVE_MULTIPLIER;
  }
  if (input.precision) {
    speed *= PLAYER_SLOW_MOVE_MULTIPLIER;
  }
  const forwardX = Math.cos(yaw);
  const forwardZ = Math.sin(yaw);
  const rightX = -forwardZ;
  const rightZ = forwardX;
  let velocityX = forwardX * input.forward + rightX * input.strafe;
  let velocityZ = forwardZ * input.forward + rightZ * input.strafe;
  const length = Math.hypot(velocityX, velocityZ);
  if (length === 0) {
    return [0, 0, 0];
  }
  velocityX = velocityX / length * speed;
  velocityZ = velocityZ / length * speed;
  return [velocityX, 0, velocityZ];
}
