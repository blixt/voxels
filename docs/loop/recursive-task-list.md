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
2. Performance/harness: tighten the new browser chunk-cache reuse benchmark until it becomes a trustworthy acceptance check, then add persisted column/region summaries for far-field and Y-range work.
3. Feature: deepen caves and below-ground progression so underground biome families matter beyond surface hints.
4. Performance/harness: split browser persistence into OPFS payloads plus IndexedDB metadata only if the benchmarked cache-reuse path is stable enough to measure cold vs warm behavior.
5. Feature: build the next interaction layer on top of the new edit seam with lightweight persistence-ready tooling such as inventory-aware interaction smoke probes, structured edit snapshots, or early gather/build affordances beyond raw break/place.
6. Performance/harness: revisit renderer sync/upload, async mesh worker sizing, and summary-driven far-field inputs after the persistence/surface-summary path is measurable.
7. Refresh this file with a fresh six-task list that still alternates `feature -> performance/harness` and still ends with another refresh task.

## Acceptance checklist per slice

- Run the smallest relevant focused tests.
- Run `mise run build`.
- Run `mise run cycle-bench` unless the slice is so narrow that the command would add no signal.
- If the slice affects live walking or rendering, add a Chrome benchmark and/or Chrome trace result to `verification.md`.
- Update `plan.md`, `progress.md`, and any design notes that changed.
- Commit the slice before starting the next one.
