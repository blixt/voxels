import { existsSync } from "node:fs";
import { resolve } from "node:path";
import { buildCloudflareLocalEnv, rootDir } from "./cf-env.mjs";
import { mirrorOwnedChildExit, spawnOwnedChild } from "./child-process-exit.ts";

// Run the locally-installed Wrangler under this repo's directory-local
// Cloudflare auth (see cf-env.mjs).

const wranglerBin = resolve(rootDir, "node_modules", "wrangler", "bin", "wrangler.js");

if (!existsSync(wranglerBin)) {
  console.error("Missing local Wrangler install. Run `vp install` first.");
  process.exit(1);
}

const child = spawnOwnedChild(process.execPath, [wranglerBin, ...process.argv.slice(2)], {
  cwd: rootDir,
  env: buildCloudflareLocalEnv(),
  stdio: "inherit",
});

mirrorOwnedChildExit(child);
