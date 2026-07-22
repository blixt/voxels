# Deployment

The initial production topology deliberately has one authority for one world:

- Cloudflare Worker `voxels` serves the Vite/WASM assets at `https://voxels.lol` and exposes only
  the same-origin `POST /api/session` credential endpoint.
- Fly app `voxels-world-blixt` runs one `shared-cpu-2x` Machine with 2 GB RAM in `cdg`.
- The encrypted `world_data` volume is mounted at `/data`; automatic snapshots are disabled.
- Fly Proxy terminates TLS for `wss://voxels-world-blixt.fly.dev` and forwards both WebSockets to
  the Rust service on port 8080.

`auto_stop_machines` uses a full stop, `auto_start_machines` is enabled, and
`min_machines_running` is zero. Fly Proxy can therefore stop the only Machine after several idle
minutes and cold-start it for the next request; an open WebSocket keeps it active. A Fly Volume
belongs to one Machine and region, so this configuration is intentionally not highly available.
Automatic volume snapshots are disabled, so arrange a separate backup before treating the world as
durable production data.

## Authentication and secrets

Cloudflare and Fly must hold the same `VOXELS_SESSION_SIGNING_KEY`. Cloudflare also holds a separate
`VOXELS_IDENTITY_SIGNING_KEY` so routine session-key rotation does not reset durable browser/player
IDs. Both keys must contain at least 32 random bytes and must never be committed, printed, or shipped
in browser assets. Create the identity key once:

```sh
identity_signing_key="$(openssl rand -base64 48 | tr -d '\n')"
printf 'VOXELS_IDENTITY_SIGNING_KEY=%s\n' "$identity_signing_key" \
  | vp run wrangler secret bulk
unset identity_signing_key
```

To rotate the shared session key:

```sh
session_signing_key="$(openssl rand -base64 48 | tr -d '\n')"
printf 'VOXELS_SESSION_SIGNING_KEY=%s\n' "$session_signing_key" \
  | fly secrets import --app voxels-world-blixt --stage
printf 'VOXELS_SESSION_SIGNING_KEY=%s\n' "$session_signing_key" \
  | vp run wrangler secret bulk
unset session_signing_key
```

The next Fly deploy activates its staged secret. Existing short-lived session tokens become invalid,
but browser identity credentials remain valid. Schedule rotation as a brief authentication
maintenance window because Cloudflare and Fly will disagree between the secret update and completed
Fly deploy. Rotating `VOXELS_IDENTITY_SIGNING_KEY` intentionally resets anonymous identities unless a
migration or previous-key verification path is deployed first.

New identity issuance is limited per client address at the Worker edge. Refreshes with an existing
credential do not consume that stricter quota. Session tokens live for 12 hours; the browser reloads
five minutes before expiry so later WebSocket reconnects use a fresh token.

Cloudflare authentication is directory-local. Copy `.env.cloudflare.local.example` to
`.env.cloudflare.local` for an API token, or run `vp run wrangler login`; both the real env file and
Wrangler state are git-ignored. Verify accounts before a release:

```sh
vp run wrangler whoami
fly auth whoami
```

## Release

Run the repository gate and validate both provider configurations before changing live traffic:

```sh
vp install --frozen-lockfile
vp run verify
vp run build:production
vp run wrangler deploy --dry-run
fly config validate -c fly.toml
```

Deploy the server first, confirm its health, and then publish the browser/Worker bundle:

```sh
fly deploy --config fly.toml --remote-only --ha=false --yes --wait-timeout 10m
curl --fail https://voxels-world-blixt.fly.dev/healthz
vp run deploy
curl --fail https://voxels.lol/
```

`vp run deploy` always rebuilds with `config/client.production.toml`; ordinary `vp build` retains the
local development endpoints. The public client obtains a Worker-signed WebSocket subprotocol token
before the TOML reaches Rust. The server verifies that token and rejects an opening world or presence
frame whose browser/player IDs do not match its signed claims.

## Operations and rollback

Useful read-only checks are:

```sh
fly status --app voxels-world-blixt
fly checks list --app voxels-world-blixt
fly volumes list --app voxels-world-blixt
fly secrets list --app voxels-world-blixt
fly logs --app voxels-world-blixt --no-tail
vp run wrangler versions list
```

Fly retains release history for `fly releases` and `fly releases rollback`. Cloudflare Worker
versions are visible through Wrangler and the dashboard. Roll back the Worker and Fly independently:
the website publish succeeding does not prove the server deploy or its volume migration succeeded.
Never delete or replace `world_data` as part of a code rollback. Automatic snapshots are disabled;
if a suitable manual or older snapshot exists, restore it into a new volume and inspect it before
changing the production mount.
