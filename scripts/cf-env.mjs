import { existsSync, mkdirSync, readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

// Directory-local Cloudflare auth. Wrangler (and the @cloudflare/vite-plugin
// remote proxy it spawns) reads its OAuth/config from XDG_CONFIG_HOME, so we
// point all four XDG dirs at .wrangler-local/. This keeps this repo's Cloudflare
// account separate from whatever account is logged in globally — `wrangler
// login` run through this env writes the token under .wrangler-local. Global
// CLOUDFLARE_*/CF_* env vars and ambient XDG paths are ignored so they cannot
// override the local account (set VOXELS_WRANGLER_INHERIT_AUTH=true to keep them).

export const rootDir = resolve(dirname(fileURLToPath(import.meta.url)), "..");

const localDir = resolve(rootDir, ".wrangler-local");
const localEnvPath = resolve(rootDir, ".env.cloudflare.local");
const cloudflareAuthEnvNames = [
  "CF_API_KEY",
  "CF_API_TOKEN",
  "CF_EMAIL",
  "CLOUDFLARE_ACCESS_CLIENT_ID",
  "CLOUDFLARE_ACCESS_CLIENT_SECRET",
  "CLOUDFLARE_ACCOUNT_ID",
  "CLOUDFLARE_API_KEY",
  "CLOUDFLARE_API_TOKEN",
  "CLOUDFLARE_API_USER_SERVICE_KEY",
  "CLOUDFLARE_EMAIL",
];

function parseEnvLine(line) {
  const trimmed = line.trim();

  if (!trimmed || trimmed.startsWith("#")) {
    return null;
  }

  const separator = trimmed.indexOf("=");

  if (separator <= 0) {
    return null;
  }

  const key = trimmed.slice(0, separator).trim();
  let value = trimmed.slice(separator + 1).trim();

  if (
    (value.startsWith('"') && value.endsWith('"')) ||
    (value.startsWith("'") && value.endsWith("'"))
  ) {
    value = value.slice(1, -1);
  }

  return key ? [key, value] : null;
}

export function buildCloudflareLocalEnv() {
  mkdirSync(localDir, { recursive: true });

  const inheritAuth = process.env.VOXELS_WRANGLER_INHERIT_AUTH === "true";
  const env = {
    ...process.env,
    XDG_CACHE_HOME:
      inheritAuth && process.env.XDG_CACHE_HOME
        ? process.env.XDG_CACHE_HOME
        : resolve(localDir, "xdg-cache"),
    XDG_CONFIG_HOME:
      inheritAuth && process.env.XDG_CONFIG_HOME
        ? process.env.XDG_CONFIG_HOME
        : resolve(localDir, "xdg-config"),
    XDG_DATA_HOME:
      inheritAuth && process.env.XDG_DATA_HOME
        ? process.env.XDG_DATA_HOME
        : resolve(localDir, "xdg-data"),
    XDG_STATE_HOME:
      inheritAuth && process.env.XDG_STATE_HOME
        ? process.env.XDG_STATE_HOME
        : resolve(localDir, "xdg-state"),
  };

  if (!inheritAuth) {
    for (const name of cloudflareAuthEnvNames) {
      delete env[name];
    }
  }

  if (existsSync(localEnvPath)) {
    for (const line of readFileSync(localEnvPath, "utf8").split(/\r?\n/)) {
      const entry = parseEnvLine(line);

      if (entry && !(entry[0] in env)) {
        env[entry[0]] = entry[1];
      }
    }
  }

  return env;
}
