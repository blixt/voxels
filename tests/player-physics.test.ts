import { expect, test } from "bun:test";

import {
  createPlayerState,
  getPlayerEyePosition,
  PLAYER_MAX_STEP_HEIGHT,
  stepPlayer,
} from "../src/engine/player-physics.ts";
import { VoxelWorld } from "../src/engine/world.ts";

test("player falls onto flat ground and becomes grounded", () => {
  const world = new VoxelWorld({ width: 128, height: 128, depth: 128 }, 32, [0, 0xff8899aa]);
  world.fillBox(0, 0, 0, 128, 1, 128, 1);
  const player = createPlayerState([48, 4, 48]);

  const result = stepPlayer(world, player, 0, idleInput(), 0.3);

  expect(result.grounded).toBe(true);
  expect(player.grounded).toBe(true);
  expect(player.feetPosition[1]).toBe(1);
  expect(player.velocity[1]).toBe(0);
});

test("player movement is blocked by solid voxels", () => {
  const world = new VoxelWorld({ width: 160, height: 128, depth: 128 }, 32, [0, 0xff8899aa]);
  world.fillBox(0, 0, 0, 160, 1, 128, 1);
  world.fillBox(20, 1, 0, 21, 64, 128, 1);
  const player = createPlayerState([10, 1, 48], { grounded: true });

  const result = stepPlayer(world, player, 0, { ...idleInput(), forward: 1 }, 0.2);

  expect(result.collidedX).toBe(true);
  expect(player.feetPosition[0]).toBe(17);
});

test("player can only jump while grounded", () => {
  const world = new VoxelWorld({ width: 128, height: 128, depth: 128 }, 32, [0, 0xff8899aa]);
  world.fillBox(0, 0, 0, 128, 1, 128, 1);
  const groundedPlayer = createPlayerState([48, 1, 48], { grounded: true });
  const airbornePlayer = createPlayerState([48, 40, 48]);

  stepPlayer(world, groundedPlayer, 0, { ...idleInput(), jump: true }, 0.016);
  stepPlayer(world, airbornePlayer, 0, { ...idleInput(), jump: true }, 0.016);

  expect(groundedPlayer.velocity[1]).toBeGreaterThan(0);
  expect(groundedPlayer.grounded).toBe(false);
  expect(airbornePlayer.velocity[1]).toBeLessThan(0);
});

test("player eye position is derived from feet position", () => {
  const player = createPlayerState([10, 20, 30]);

  expect(getPlayerEyePosition(player)).toEqual([10, 36.8, 30]);
});

test("player walks up a 3-voxel step without jumping", () => {
  const world = new VoxelWorld({ width: 64, height: 64, depth: 64 }, 16, [0, 0xff8899aa]);
  world.fillBox(0, 0, 0, 64, 1, 64, 1);
  world.fillBox(20, 1, 0, 64, 1 + PLAYER_MAX_STEP_HEIGHT, 64, 1);
  const player = createPlayerState([16, 1, 32], { grounded: true });

  const result = stepPlayer(world, player, 0, { ...idleInput(), forward: 1 }, 0.12);

  expect(result.collidedX).toBe(false);
  expect(result.grounded).toBe(true);
  expect(player.feetPosition[0]).toBeGreaterThan(20);
  expect(player.feetPosition[1]).toBe(1 + PLAYER_MAX_STEP_HEIGHT);
});

test("player does not auto-step onto obstacles higher than 3 voxels", () => {
  const world = new VoxelWorld({ width: 64, height: 64, depth: 64 }, 16, [0, 0xff8899aa]);
  world.fillBox(0, 0, 0, 64, 1, 64, 1);
  world.fillBox(20, 1, 0, 64, 2 + PLAYER_MAX_STEP_HEIGHT, 64, 1);
  const player = createPlayerState([16, 1, 32], { grounded: true });

  const result = stepPlayer(world, player, 0, { ...idleInput(), forward: 1 }, 0.12);

  expect(result.collidedX).toBe(true);
  expect(player.feetPosition[0]).toBe(17);
  expect(player.feetPosition[1]).toBe(1);
});

function idleInput() {
  return {
    forward: 0,
    strafe: 0,
    jump: false,
    sprint: false,
    precision: false,
  };
}
