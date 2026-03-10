# Research / Build / Test Loop

This repository keeps the design loop in versioned documents instead of ad-hoc notes.

- `../agent-playbook.md`: repo-specific guidance for agent-driven research, parallel work, verification, and worktree usage
- `../20260311-voxel-research.md`: broader architecture and literature survey for future engine work
- `../roadmap.md`: staged path from the current engine baseline to the shared persistent voxel game target
- `plan.md`: current implementation plan and next milestones
- `research.md`: external sources, constraints, and technical decisions
- `progress.md`: chronological implementation log
- `hypotheses.md`: hypothesis grid, tiny probes, and outcomes for renderer errors
- `verification.md`: commands, browser checks, and benchmark runs

Loop:

1. Capture constraints and current browser/runtime facts in `research.md`.
2. Record the active build plan in `plan.md`.
3. Implement the next smallest slice and log major decisions in `progress.md`.
4. Run `mise run test`, exercise `/bench`, and record the outcome in `verification.md`.
5. Run `mise run profile` when browser automation is unavailable or when I need repeatable local build/mesh timings with an explicit warm-up pass.
6. Update `plan.md` with the next smallest verified slice toward the game/runtime target.
