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

1. Performance/harness: attack the post-worker throughput bottleneck by measuring and reducing detailed chunk adoption/meshing latency while the player moves, and keep it only if route traces or cycle-bench data improve without reopening hole signals.
2. Feature: add biome-aware points of interest and cave-side setpieces that make the discovery journal pay off beyond counts alone.
3. Performance/harness: add a trace-friendly build/report mode so browser route traces keep meaningful symbol names and can be folded into a broader acceptance battery.
4. Feature: deepen caves and below-ground progression so underground biome families matter beyond surface hints.
5. Performance/harness: extend the off-main-thread path from generation into meshing prep or chunk-adoption batching, but only if route A/B traces improve again.
6. Feature: build the next interaction layer on top of the new edit seam with lightweight persistence-ready tooling such as inventory-aware interaction smoke probes, structured edit snapshots, or early gather/build affordances beyond raw break/place.
7. Refresh this file with a fresh six-task list that still alternates `feature -> performance/harness` and still ends with another refresh task.

## Acceptance checklist per slice

- Run the smallest relevant focused tests.
- Run `mise run build`.
- Run `mise run cycle-bench` unless the slice is so narrow that the command would add no signal.
- If the slice affects live walking or rendering, add a Chrome benchmark and/or Chrome trace result to `verification.md`.
- Update `plan.md`, `progress.md`, and any design notes that changed.
- Commit the slice before starting the next one.
