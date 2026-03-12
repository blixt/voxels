# Recursive Task List

This file is the durable "keep going" loop for the repo. It exists so a future slice does not depend on short-term memory or on rereading the whole project history.

## Loop contract

1. Alternate `feature -> performance/harness -> feature -> performance/harness`.
2. Every slice must go through `research -> plan -> implement -> verify -> document -> commit`.
3. Every feature slice must leave behind at least one deterministic check, probe, or automation seam.
4. Every performance slice must leave behind at least one measurable benchmark/tracing delta or a documented rejection.
5. Prefer rewriting/removing over layering. If a slice only adds indirection, remove it.
6. Use `mise run cycle-bench` as the default command-line acceptance gate for multi-front work.
7. If a slice touches live movement, streaming, or rendering behavior, add a browser or trace follow-up in addition to `cycle-bench`.
8. When a task finishes, archive the outcome in `progress.md` and replace the finished task with the next best item while preserving alternation.

## Current cycle

1. Performance/harness: automate a standard browser route + trace run so the repo can compare live-movement CPU, allocations, long tasks, and hole diagnostics from one command.
2. Feature: deepen biome identity with denser local forest canopies, more dramatic landmark clustering, and stronger below-ground biome/object signatures.
3. Performance/harness: prototype a lower-main-thread streaming path, starting with workerized or staged chunk generation/meshing experiments that can be A/B tested against the route benchmark.
4. Feature: implement the first real gather/build loop with inventory pickup, placement, and persistence-ready edit records.
5. Performance/harness: replace more of the ad hoc far-field path with a more principled LOD experiment inspired by clipmaps and seam-stitching research, and keep it only if the route benchmark and seam probes improve.
6. Feature: add biome-aware points of interest and cave-side setpieces that make the new discovery journal pay off beyond counts alone.
7. Refresh this file with a fresh six-task list that still alternates `feature -> performance/harness` and still ends with another refresh task.

## Acceptance checklist per slice

- Run the smallest relevant focused tests.
- Run `mise run build`.
- Run `mise run cycle-bench` unless the slice is so narrow that the command would add no signal.
- If the slice affects live walking or rendering, add a Chrome benchmark and/or Chrome trace result to `verification.md`.
- Update `plan.md`, `progress.md`, and any design notes that changed.
- Commit the slice before starting the next one.
