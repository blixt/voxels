import { expect, test } from "bun:test";

import {
  createPlayerState,
  getPlayerEyePosition,
  stepPlayer,
} from "../src/engine/player-physics.ts";
import { VoxelWorld } from "../src/engine/world.ts";

test("player falls onto flat ground and becomes grounded", () => {
  const world = new VoxelWorld({ width: 128, height: 128, depth: 128 }, 32, [0, 0xff8899aa]);
  world.fillBox(0, 0, 0, 128, 1, 128, 1);
  const player = createPlayerState([48, 20, 48]);

  const result = stepPlayer(world, player, 0, idleInput(), 0.2);

  expect(result.grounded).toBe(true);
  expect(player.grounded).toBe(true);
  expect(player.feetPosition[1]).toBe(1);
  expect(player.velocity[1]).toBe(0);
});

test("player movement is blocked by solid voxels", () => {
  const world = new VoxelWorld({ width: 160, height: 128, depth: 128 }, 32, [0, 0xff8899aa]);
  world.fillBox(0, 0, 0, 160, 1, 128, 1);
  world.fillBox(80, 1, 0, 81, 64, 128, 1);
  const player = createPlayerState([40, 1, 48], { grounded: true });

  const result = stepPlayer(world, player, 0, { ...idleInput(), forward: 1 }, 0.2);

  expect(result.collidedX).toBe(true);
  expect(player.feetPosition[0]).toBe(50);
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

  expect(getPlayerEyePosition(player)).toEqual([10, 188, 30]);
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
