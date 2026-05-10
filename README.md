# Voxels

WebGPU voxel exploration game built with Bun and TypeScript.

## Workflow

1. Install tooling with `mise install`.
2. Install dependencies with `bun install`.
3. Start the app with `mise run dev`.
4. Open `http://localhost:3000/` for the game.
5. Build production output with `mise run build`, then run it with `mise run serve`.

`mise run dev` uses Bun's hot-reload/full-stack dev path, so edits to the server, page shell, CSS, and browser entrypoints update without manual cache-busting.

## Current Direction

- The root route boots directly into the exploratory island game.
- The world is procedurally generated and lazily streamed around the player.
- The active direction is a coherent island RPG world with authored landmarks, biomes, caves, ambient systems, mobs, vegetation, loot, skills, and exploration-first interaction loops.

## Game Automation

- `window.__VOXELS_GAME__.snapshot()`
- `window.__VOXELS_GAME__.snapshotResidentWorld()`
- `window.__VOXELS_GAME__.teleport(x, y, z)`
- `window.__VOXELS_GAME__.teleportAndSettle(x, y, z, { radiusChunks })`
- `window.__VOXELS_GAME__.requestPointerLock()`
- `window.__VOXELS_GAME__.setViewDistance(chunks)`
- `window.__VOXELS_GAME__.forceResidencyUpdate()`
- `window.__VOXELS_GAME__.getDiscoveryJournal()`
- `window.__VOXELS_GAME__.resetDiscoveryJournal()`
