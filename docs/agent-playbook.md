# Agent Playbook

This document captures what worked, what did not work, and what this repository should optimize for if the main goal is fast, independent, agent-driven exploration of technical directions.

It is intentionally practical. It is based on the actual history of this repo so far, not on generic process advice.

## Core goals

- Make it cheap to explore many technical directions.
- Make it easy to reject bad directions quickly.
- Make correctness less dependent on human visual inspection.
- Make performance decisions based on the target runtime, not on guesswork.
- Keep the repo safe for parallel work by multiple agents.

## What worked

### Start from primitives before big scenes

- Tiny validation scenes were far more useful than jumping directly into the full terrain scene.
- Primitive checks such as a single voxel, a small cube, and a compact block arrangement caught renderer issues much faster than looking at a large scene.
- Small, asymmetric cases are especially valuable because they expose axis swaps, winding mistakes, depth mistakes, and culling errors quickly.

### Separate correctness work from performance work

- Treating correctness and performance as separate tracks was a major improvement.
- Performance scenes should not pretend they are visually validated if they are not.
- Validation scenes should be cheap, deterministic, and runnable often.

### Use a hypothesis grid, not ad-hoc debugging

- The renderer became easier to debug once work shifted from "guess and tweak" to "write down hypotheses, define tiny probes, reject quickly".
- `docs/loop/hypotheses.md` is worth maintaining because it prevents repeated dead-end work.
- A broad hypothesis list up front is better than locking onto the first plausible explanation.

### Measure first-frame and warm-frame cost separately

- Separating first-frame CPU/GPU and warm-frame CPU/GPU was a real improvement.
- Separating sync/upload/encode work made hidden costs visible that were previously buried inside one frame average.
- This matters because streaming, live edits, and scene loads are not the same problem as steady-state rendering.

### Use Chrome as the final performance oracle

- Warmed local Bun/JSC profiling is useful for fast local iteration.
- Chrome 146/V8 is the actual target runtime, so accept/reject decisions for renderer and mesher changes should be made there.
- A change can look promising in a local microbench and still be the wrong choice in Chrome.

### Use isolated browser contexts for benchmark runs

- Fresh isolated Chrome contexts were much more reliable than reusing a page that had already run other scenarios.
- Overlapping or sequential runs in the same page can contaminate state and make the results untrustworthy.
- Clean page state is part of the benchmark contract.

### Use cache-busting and no-store semantics

- Stale browser bundles caused a real false debugging trail.
- Serving HTML, CSS, and bundles with `no-store` and cache-busted URLs was not optional; it was necessary for trustworthy browser verification.

### Prefer measured rewrites over additive layers

- Performance work improved once the default became "rewrite/remove" rather than "add another helper or wrapper".
- Dead code and abandoned instrumentation should be removed quickly instead of kept around "just in case".
- Git is the safety net; the worktree should stay lean.

### Split runtime modes instead of forcing one controller to do everything

- The benchmark/orbit controller stack was a useful foundation, but it is the wrong place to bolt on a first-person game mode.
- A dedicated game controller is safer than turning one controller into a large mode switch.
- This repo should prefer narrow runtime boundaries over broad "unified" controllers when the interaction model changes fundamentally.

### Cut architectural seams before adding the harder feature

- For the infinite-world pivot, the correct first refactor was the world-access boundary, not a premature streaming implementation.
- Refactoring meshing and rendering onto a resident-chunk interface keeps the benchmark path stable and narrows the next problem.
- When a system has a hard future requirement, prefer the seam that reduces coupling first and the feature second.

### Commit every real unit of progress

- Frequent commits made it much easier to compare strategies, explain the history, and keep experimental work bounded.
- Small commits also made it easier to drop a bad direction without collateral damage.

### Keep local profiling scriptable

- A repo-local profiling script was much better than relying on memory or hand-run console snippets.
- `mise run profile` is useful because it makes local iteration cheap when browser automation is unavailable or too slow for every loop.

## What did not work

### Starting too deep in the stack

- Looking at a large terrain render before primitive correctness was locked down was a mistake.
- It is too hard to tell whether a problem is projection, culling, meshing, depth, lighting, or just scene complexity.

### Trusting one runtime too much

- Bun/JSC-only microbench results were not a safe final oracle for WebGPU engine decisions.
- They are still useful, but only as a screen before the Chrome run.

### Mixing too many goals into one benchmark

- When one run tries to cover correctness, performance, and scene loading all at once, the result is harder to interpret.
- Separate "validation-only", "steady-state performance", and "first-frame/upload" modes are better.

### Reusing dirty browser state

- Running different scenarios back-to-back in the same page without resetting state produced confusing results.
- Reusing a page should be treated as suspicious unless the harness explicitly guarantees clean state.

### Trusting privileged browser APIs to automation

- Synthetic automation is not a reliable oracle for privileged browser features such as Pointer Lock.
- The right response is not to guess that the feature works; it is to expose a debug/status surface so the success path and failure path are both inspectable.
- Game and benchmark routes should keep scriptable state surfaces even when a real human gesture is still required for the final interaction.

### Letting instrumentation drift half-finished

- Partially wired metrics created type drift and verification friction.
- If new metrics are introduced, they should either be completed end-to-end or reverted quickly.

### Choosing strategies before doing wide research

- Early on, too much attention went to specific local explanations before the search space had been mapped properly.
- Broad research and broad error-case enumeration should happen before committing to a fix direction.

## Recommended agent workflow

1. Define the question narrowly.
2. Build a broad hypothesis list before changing code.
3. Add or identify the cheapest possible verification cases.
4. Run a baseline in the target runtime.
5. Explore in parallel only where write scopes are clearly separated.
6. Keep the smallest promising change.
7. Re-verify in layers: unit, local profiler, clean Chrome.
8. Document what was learned.
9. Commit the unit.

## Research strategy that works better

### Start broad

- Before optimizing, enumerate the likely failure categories.
- For rendering, that can include projection, depth, culling, readback, stale bundles, meshing, scene setup, and driver/runtime behavior.
- For performance, that can include scene build, meshing, upload, draw submission, overdraw, validation overhead, and cache effects.

### Search for failure modes, not just techniques

- Broad research should include "what goes wrong" and not only "how to go faster".
- It is often more useful to search for error classes, bottleneck patterns, and tradeoffs than for a named technique.

### Convert research into hypotheses immediately

- Every promising source should become one or more concrete local hypotheses.
- Every hypothesis should have a tiny verification case and a kill condition.

### Keep a cheap kill threshold

- If a strategy fails the tiny case, drop it early.
- If a strategy improves one metric but regresses the acceptance metric in Chrome, drop it or isolate it further.

## How to parallelize work better

### Parallelize information gathering first

- One agent should inspect likely code hotspots.
- One agent should inspect harness blind spots.
- One agent should gather external references or error cases.
- These are good parallel tasks because they do not fight over files.

### Parallelize implementation only with disjoint ownership

- Split by write scope, not by vague responsibility.
- Good splits include:
  - renderer instrumentation
  - world/scenes generation
  - docs and process notes
  - verification scripts
- Avoid having multiple agents edit the same hot path unless absolutely necessary.

### Parallelize verification lanes

- Run unit tests, local profiler runs, and browser verification as separate lanes.
- If a change fails an early lane, stop before paying for the more expensive ones.

### Use a baseline lane

- One lane should always preserve the clean comparison point.
- This is where git worktrees are especially useful.

## Git worktrees

Git worktrees deserve to be part of the standard workflow for this repo.

### Why they help

- They make A/B comparisons against `HEAD` cheap.
- They allow multiple agents to work in parallel without stomping on the same checkout.
- They make it easier to run separate servers or scripts per experiment.
- They reduce the temptation to keep unrelated experiments mixed in one worktree.

### Recommended uses

- Keep one clean baseline worktree.
- Create one worktree per serious experiment branch.
- Use detached worktrees for direct baseline measurements when a branch-to-`HEAD` comparison is needed.
- Name worktrees by intent, not by person, for example:
  - `voxels-mesh-ab`
  - `voxels-streaming-probe`
  - `voxels-lighting-spike`

### What to compare in worktrees

- Build time
- Mesh time
- First-frame upload/sync cost
- Warm-frame cost
- Validation checksums and image metrics
- Output artifacts when relevant

## Verification rules worth keeping

### Verification stack

- Unit tests protect invariants.
- Tiny validation scenes protect rendering principles.
- Local profiles protect iteration speed.
- Clean isolated Chrome runs protect final decisions.

### Acceptance order

1. Unit tests
2. Local profile or tiny probe
3. Clean Chrome verification
4. Docs update
5. Commit

### Cases that should stay cheap

- Single voxel
- Small cube
- Chunk-boundary visibility
- Edit add/remove near boundaries
- Validation image diff
- First-frame upload behavior
- First-person camera yaw/pitch invariants
- Center-screen pick ray sanity
- Inventory stack-limit rules
- Deterministic chunk-generation probes

## Advice that would have helped at the start

- Add validation scenes before trying to read large scene images.
- Add first-frame vs warm-frame metrics earlier.
- Treat stale browser state as a likely bug source from day one.
- Decide early which runtime is the final oracle.
- Use worktrees earlier for A/B comparison instead of doing mental comparisons across changing worktrees.
- Write down rejected hypotheses immediately so they are not rediscovered later.
- Commit more often than feels necessary.

## How the repo can become more agent-friendly

### Server and harness improvements

- Add a structured benchmark output mode that returns machine-readable JSON without needing DOM scraping.
- Add a validation-only mode that renders one deterministic frame and returns artifacts plus metrics.
- Add run IDs, commit SHAs, bundle version, Chrome version, and scenario metadata to benchmark output.
- Add an explicit "page ready" signal for automation so agents do not guess when the harness is initialized.
- Add artifact output directories for screenshot diffs, benchmark JSON, and trace summaries.
- Expose game-state JSON and deterministic debug hooks on `/` the same way `/bench` exposes benchmark APIs.
- Add a scenario manifest file so scenes, validation scenes, and stress scenes are discoverable without parsing UI code.
- Add a headless Chrome runner script for repeatable browser verification outside interactive sessions.
- Add benchmark modes for:
  - first-frame only
  - warm steady-state only
  - validation only
  - stream-in / chunk churn
  - edit burst

### Multi-agent-friendly execution

- Support per-run ports or per-run server IDs so multiple worktrees can be exercised at the same time without collisions.
- Stamp every benchmark result with the active bundle version so stale output is easier to detect.
- Prefer deterministic inputs and camera presets so different agents can compare results directly.
- Keep benchmark control scriptable through JS APIs and CLI entry points.
- Make the harness easy to reset between runs.

### Multi-agent-friendly implementation

- Keep hot paths small and well-factored so one agent can replace a strategy without broad repo churn.
- Prefer explicit data flow over hidden shared state.
- Keep scenario generation, meshing, renderer upload, and validation logic separable.
- Keep docs current so an agent can build context without rereading the entire git history.

## Better ways to test many strategies in parallel

### Strategy matrix

- Define a matrix of candidate strategies before implementation.
- Example performance matrix:
  - current baseline
  - data-layout rewrite
  - allocation rewrite
  - upload-path rewrite
  - workerization
- Give each strategy the same measurement contract.

### Shared comparison harness

- Use one shared script or endpoint so every branch reports the same fields.
- Avoid one-off console snippets when comparing serious options.

### Wide research before narrowing

- Split broad research across agents:
  - one on primary technical references
  - one on practical engine writeups
  - one on failure modes and edge cases
  - one on verification design
- Only narrow once the search space is mapped and translated into local hypotheses.

### Keep a branch disposable until it wins

- Exploration branches should be cheap to delete.
- A strategy should earn its way into the mainline by passing the acceptance stack, not by accumulating time investment.

## Good next repo improvements

- Add a headless browser benchmark runner that writes JSON artifacts to disk.
- Add `validation-only` and `json-only` harness modes.
- Add scenario manifests and benchmark baselines in-repo.
- Add worktree-aware helper scripts for A/B comparison.
- Add streaming and LOD validation scenes before those systems land.
- Add explicit support for artifact collection per run so agents can compare output without manual screenshots.
