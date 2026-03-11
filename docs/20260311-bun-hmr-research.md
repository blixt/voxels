# Bun HMR / Live-Edit Research

Date: 2026-03-11

## Goal

Find the best Bun-native development loop for this repo so live editing does not depend on cache-busting, manual rebuilds, or repeatedly starting fresh server instances.

## Current repo state

The current server path is intentionally simple, but it is the wrong shape for fast iteration:

- `src/server.ts` runs `buildClientBundles()` at startup.
- browser bundles are written to `public/build`
- HTML is generated as strings in `src/server/templates.ts`
- the server compensates with `Cache-Control: no-store` and query-string cache busting

That makes stale assets less likely, but it does not give us a true live-edit loop. If the server code or templates drift from the running process, the safest verification path still becomes "new port, fresh tab, try again".

## Broad option search

### Option 1: keep the current startup build and just use `bun --hot`

Verdict: not enough

- `bun --hot src/server.ts` would improve server-code reload speed.
- It does not solve the bigger problem here: frontend assets are still built once into `public/build`.
- HTML string templates are still outside Bun's frontend dev-server model.

This is better than full restarts, but it still leaves client-bundle freshness as a separate problem.

### Option 2: keep manual bundling and run `bun build --watch`

Verdict: still not the best fit

- This can rebuild browser bundles when files change.
- It does not give Bun-native browser HMR for the client app.
- It adds another watcher/process and still leaves HTML/template refresh behavior awkward.

This is a tolerable fallback, not the best Bun-native loop.

### Option 3: move the repo onto Bun's HTML-import full-stack dev server

Verdict: best dev path

Official Bun docs now support the exact model we want:

- import `.html` files directly from server code
- route them from `Bun.serve()`
- enable `development: true` or `development: { hmr: true, console: true }`
- run the server with `bun --hot`

In that mode Bun will:

- re-bundle assets on each request to the HTML entry
- enable browser HMR
- disable minification
- keep sourcemaps on
- stream browser console logs back to the terminal

This is the first option that addresses both halves of the problem:

- server-side live editing via `bun --hot`
- client-side live editing via Bun's dev server and HMR

### Option 4: use `bun ./index.html` directly

Verdict: useful for tiny frontend-only prototypes, not for this repo

The repo already has:

- a custom server
- multiple routes
- browser automation entry points
- a likely future multiplayer/API backend

Running a standalone HTML dev server would split the app in the wrong direction. The better fit is Bun's full-stack dev-server path inside the existing `Bun.serve()` process.

## Recommended repo direction

### Development

Use Bun's full-stack HTML-import path for `/` and `/bench`.

Recommended shape:

1. Create static HTML entry files for the game and benchmark shells.
2. Import those HTML files from `src/server.ts`.
3. Serve them directly from `Bun.serve({ routes })`.
4. Enable:

```ts
development: {
  hmr: true,
  console: true,
}
```

5. Run the dev server with:

```sh
bun --hot src/server.ts
```

### Production

Keep development and production clearly separate.

Recommended shape:

- dev: runtime bundling + HMR through Bun's HTML-import dev server
- prod: ahead-of-time build with `bun build --target=bun --production`

That matches Bun's current recommendation. Production should not depend on runtime rebundling or HMR.

## Why this is better for this repo

This repo is not a static site. It is a browser-heavy engine/game harness where the expensive bugs are usually:

- stale frontend code
- stale HTML/template code
- event listeners or render loops surviving edits
- multiple localhost servers making it hard to tell what code the browser is actually running

The Bun-native full-stack dev path directly improves all of those:

- asset freshness is owned by Bun instead of by our handwritten cache strategy
- browser logs can flow back into the terminal via `development.console`
- server code can soft-reload without dropping the process
- client modules can accept/dispose cleanly instead of forcing constant hard refreshes

## Important caveats

### `bun --hot` is not the whole answer

Bun's runtime docs are explicit that `--hot` by itself is server-side hot reload, not browser HMR.

For this repo, the correct pairing is:

- `bun --hot` for the server process
- HTML imports plus `development.hmr` for the browser app

Using only `bun --hot` while keeping manual `public/build` output would still leave the browser freshness problem mostly unsolved.

### Global state survives `--hot`

Bun's `--hot` keeps `globalThis` alive across reloads.

That is useful for some server state, but it is also a footgun. For this repo, the safe default is:

- avoid unnecessary `globalThis` state in the dev server
- keep long-lived state explicit
- make cleanup paths obvious

### Stateful client modules need HMR teardown

This repo has modules that own:

- WebGPU devices/pipelines/buffers
- `requestAnimationFrame` loops
- DOM event listeners
- pointer-lock/mouse state

Those modules should not rely on blind re-evaluation. When the repo migrates to Bun's dev server path, `src/client/game.ts`, `src/client/bench.ts`, and any long-lived controller entrypoints should add `import.meta.hot` handling:

- `import.meta.hot.accept()` to define an HMR boundary
- `import.meta.hot.dispose(...)` to tear down RAF loops and listeners
- `import.meta.hot.data` for small amounts of state worth preserving during reload

For WebGPU specifically, the first goal is not clever state preservation. The first goal is deterministic cleanup so edits do not leave duplicate loops, listeners, or controllers alive.

## Repo-specific migration plan

### Phase 1: dev-server alignment

- Add `src/pages/game.html` and `src/pages/bench.html`.
- Move the mostly static shell markup out of `src/server/templates.ts`.
- Point the HTML files at the existing client entrypoints and stylesheet.
- Route those HTML imports directly from `src/server.ts`.
- Add a dedicated dev script that uses `bun --hot`.

Expected result:

- frontend edits should stop depending on manual cache-busting
- route-shell edits should update without fresh ports in normal development

### Phase 2: client HMR hygiene

- Add `import.meta.hot.accept()` to top-level browser entry modules.
- Add `dispose()` support to controller/bootstrap paths so the old runtime can shut down cleanly before the new module takes over.
- Make console logging deliberate so `development.console` becomes useful instead of noisy.

Expected result:

- fast edits without duplicate render loops
- better debugging from the terminal

### Phase 3: shrink the custom cache logic

After Bun's dev-server path is in place, re-check which of these are still needed in development:

- manual `assetVersion()` query strings
- explicit `/build/:file` asset serving
- broad `no-store` logic meant to fight stale manual bundles

The likely clean outcome is:

- keep dev simple and Bun-native
- keep prod explicit and cached
- delete custom dev-only cache-fighting code once it stops pulling its weight

## Recommended commands

### Current repo

Current default command:

```sh
mise run dev
```

That still follows the manual startup-bundle path today.

### Recommended future dev command

```sh
bun --hot src/server.ts
```

Wrapped in `mise`/`package.json`, that should become the normal local dev entrypoint once the HTML-import migration is complete.

### Recommended watch usage outside the app server

- use `bun --watch test` for test iteration
- prefer `bun --hot` for the dev server itself
- use `--no-clear-screen` if multiple watch processes are running and terminal output should remain readable

## Version check

As of 2026-03-11, the project-managed toolchain is current:

- Chrome stable `146.0.7680.72`
- Bun `1.3.10`
- `@types/bun` `1.3.10`
- TypeScript `5.9.3`
- `@webgpu/types` `0.1.69`
- mise `2026.3.7`

## Bottom line

For this repo, the best Bun-native answer is not "more cache-busting" and not "just add `--hot` to the current server". The best answer is:

- serve `/` and `/bench` through Bun HTML imports
- enable `development: { hmr: true, console: true }`
- run the dev server with `bun --hot`
- add explicit HMR teardown to the WebGPU/browser entrypoints
- keep production on an ahead-of-time `bun build --target=bun --production` path

That is the cleanest route to fast live editing without constantly second-guessing whether the browser is running stale code.

## Implementation follow-up

The repo now uses this development model. Three implementation details were worth recording:

- the built Bun server must be started from inside `dist/`, or its bundled HTML-import asset paths will fail to resolve
- `process.env.NODE_ENV` should be read in a runtime-bound way inside `src/server.ts`; Bun's production build can otherwise fold the expression too early and accidentally leave the built server in development mode
- self-accepting browser entry modules should stay synchronous at module level; the first `await mount...()` version triggered Bun HMR runtime errors during reload, while a synchronous module plus async controller init worked cleanly

## Sources

- Bun full-stack dev server: https://bun.sh/docs/bundler/fullstack
- Bun hot reloading / `import.meta.hot`: https://bun.sh/docs/bundler/hot-reloading
- Bun watch and `--hot`: https://bun.sh/docs/runtime/watch-mode
- Bun server / HTML imports: https://bun.sh/docs/runtime/http/server
- Bun runtime CLI: https://bun.sh/docs/cli/run
- Bun home page and current latest version: https://bun.sh/
