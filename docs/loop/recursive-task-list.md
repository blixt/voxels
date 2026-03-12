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

1. Feature: add biome-aware points of interest and cave-side setpieces that make the discovery journal pay off beyond counts alone.
2. Performance/harness: use the new saved route samples to separate main-thread vs worker cost more cleanly, then attack async transfer/GC pressure or residency/Y-range sampling if the data supports it.
3. Feature: deepen caves and below-ground progression so underground biome families matter beyond surface hints.
4. Performance/harness: extend the off-main-thread path with smarter adoption batching or dirty-queue handling only if route A/B traces improve again without reopening holes.
5. Feature: build the next interaction layer on top of the new edit seam with lightweight persistence-ready tooling such as inventory-aware interaction smoke probes, structured edit snapshots, or early gather/build affordances beyond raw break/place.
6. Performance/harness: revisit renderer sync/upload and async mesh worker sizing after the new route-sample artifacts make spike attribution less lossy.
7. Refresh this file with a fresh six-task list that still alternates `feature -> performance/harness` and still ends with another refresh task.

## Acceptance checklist per slice

- Run the smallest relevant focused tests.
- Run `mise run build`.
- Run `mise run cycle-bench` unless the slice is so narrow that the command would add no signal.
- If the slice affects live walking or rendering, add a Chrome benchmark and/or Chrome trace result to `verification.md`.
- Update `plan.md`, `progress.md`, and any design notes that changed.
- Commit the slice before starting the next one.
