# 2026-05-09 RPG Slice Audit Rubric

## Purpose

`scripts/rpg-slice-audit.ts` is a deterministic JSON audit for the current Morrowind-like game goal. It scores the full game slice by player-facing RPG progress, not by LOD internals. Render and LOD outputs still matter, but only as evidence that the slice has available performance/correctness verification.

Run:

```sh
mise exec -- bun run scripts/rpg-slice-audit.ts
```

Optional:

```sh
mise exec -- bun run scripts/rpg-slice-audit.ts -- --artifact-root=artifacts
```

## Score Bands

Each category scores `0..5`.

| Band | Meaning |
| ---: | --- |
| `0.00-1.99` | Weak or mostly absent. |
| `2.00-3.49` | Partial implementation with important gaps. |
| `3.50-3.99` | Near milestone, but still risky. |
| `4.00-5.00` | Milestone-ready for the current slice. |

## Categories

### World Definition

Measures authored RPG place structure:

- named macro regions
- route graph
- cave systems
- ambient profile coverage
- region graph coverage
- route and cave anchors
- finite island/coast samples
- route set-piece hooks

This deliberately uses `WorldAtlas` and sampling helpers as the source of truth, not LOD storage or far-render summaries.

### Exploration Loop

Measures whether the player-facing loop exists as deterministic data:

- inspect/read/use verbs
- route goal steps
- completed route goal simulation
- route journal import/export
- discovery categories
- objective progression into the deep loop

### Ambiance

Measures region identity and atmosphere:

- distinct ambient profile count
- profile coverage across regions
- color/fog/sky separation
- route context variety
- underground profile support
- screenshot evidence availability

Screenshot availability is evidence only; the score does not require a specific latest screenshot run.

### Skills And Progression

Measures RPG progression mechanics:

- four exploration skills
- discovery XP categories
- surface and underground travel XP contexts
- skill journal import/export
- skill effects that improve exploration affordances
- duplicate discovery idempotency

### Performance Evidence Availability

Measures whether the project has usable verification surfaces:

- route/view/render/RPG/browser benchmark commands
- artifact categories present when local artifacts exist
- browser benchmark evidence path
- render evidence path
- RPG verification path
- explicit non-reliance on "latest artifact" freshness for progress scoring

LOD-specific reports are allowed evidence here, but they are not the main game-progress criterion.

### Verification Coverage

Measures deterministic coverage around the slice:

- focused test file count
- pure module test count
- browser harness test coverage
- this rubric document
- fixture tests for audit scoring
- documented JSON command

## JSON Contract

The script writes a single JSON object to stdout:

- `schemaVersion`
- `goal`
- `scoringNote`
- `overallScore`
- `status`
- `subScores`
- `facts`

`subScores[].criteria[]` records each criterion's value, target, pass/fail state, and weight so future agents can see exactly why a score moved.
