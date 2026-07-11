# Repository instructions

## Tooling

This repository standardizes on Vite+.

- Use `vp install`, `vp add`, `vp remove`, and `vp update` for dependencies.
- Use `vp dev`, `vp check`, `vp test`, `vp build`, and `vp preview` directly.
- Use `vp run <task>` for project-specific tasks.
- Keep `vite-plus` and `@voidzero-dev/vite-plus-core` aligned through the pnpm catalog.
- Verify tooling changes with `vp install --frozen-lockfile`, `vp check`, `vp test`, and `vp build`.

## Architecture

- Keep portable simulation and voxel data in `core`; it must remain host-testable.
- Keep WGPU rendering in `render`; it must not name browser or wasm types.
- Keep browser/WASM glue, input decoding, and persistence in `shell`.
- Keep TypeScript limited to browser-required input, canvas, worker, and development harness code.
- Version every persisted schema and binary format before committing it.

## Local data safety

Browser OPFS data and ignored benchmark artifacts may contain valuable local worlds. Verification must
use explicit temporary databases/paths and must not reset user data.
