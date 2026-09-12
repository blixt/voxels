# Archived terrain-diffusion experiment

This document records the removed macOS/Metal diffusion provider for historical context. It is no
longer part of the workspace, is not built or fetched by `world-service`, and has no runnable
commands. Current worlds use the deterministic, versioned `procedural-v17` source described in
[`20260912-mutable-voxel-research.md`](20260912-mutable-voxel-research.md).

The old provider and its model fixtures were deliberately deleted because learned 30 m tiles were
slow to load, awkward to edit at 10 cm resolution, and unsuitable as the authoritative source for a
large mutable multiplayer world. Historical implementation details remain in repository history if
needed for comparison.
