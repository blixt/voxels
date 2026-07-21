import { existsSync } from "node:fs";
import { resolve } from "node:path";
import { spawn } from "node:child_process";
import { buildCloudflareLocalEnv, rootDir } from "./cf-env.mjs";
import { mirrorChildExit } from "./child-process-exit.mjs";

// Run the locally-installed Wrangler under this repo's directory-local
// Cloudflare auth (see cf-env.mjs).

const wranglerBin = resolve(rootDir, "node_modules", "wrangler", "bin", "wrangler.js");

if (!existsSync(wranglerBin)) {
  console.error("Missing local Wrangler install. Run `vp install` first.");
  process.exit(1);
}

const child = spawn(process.execPath, [wranglerBin, ...process.argv.slice(2)], {
  cwd: rootDir,
  env: buildCloudflareLocalEnv(),
  stdio: "inherit",
});

mirrorChildExit(child);
