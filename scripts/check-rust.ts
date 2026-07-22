import { spawn } from "node:child_process";
import { rustTool, wasmCcEnv } from "./build-wasm.ts";

const HEARTBEAT_INTERVAL_MS = 30_000;

async function cargo(label: string, args: string[], env = process.env): Promise<void> {
  const startedAt = Date.now();
  console.error(`[rust-check] ${label}`);
  await new Promise<void>((resolve, reject) => {
    const child = spawn(rustTool("cargo"), args, {
      stdio: "inherit",
      env,
    });
    const heartbeat = setInterval(() => {
      const elapsedSeconds = Math.round((Date.now() - startedAt) / 1_000);
      console.error(`[rust-check] ${label} still running (${elapsedSeconds}s)`);
    }, HEARTBEAT_INTERVAL_MS);
    child.once("error", (error) => {
      clearInterval(heartbeat);
      reject(error);
    });
    child.once("exit", (code, signal) => {
      clearInterval(heartbeat);
      if (code === 0) {
        resolve();
        return;
      }
      const outcome = signal === null ? `exit code ${String(code)}` : `signal ${signal}`;
      reject(new Error(`${label} failed with ${outcome}`));
    });
  });
  const elapsedSeconds = ((Date.now() - startedAt) / 1_000).toFixed(1);
  console.error(`[rust-check] ${label} passed in ${elapsedSeconds}s`);
}

await cargo("Rust formatting", ["fmt", "--all", "--", "--check"]);
await cargo("workspace tests", ["test", "--workspace"]);
await cargo("world-service automation tests", [
  "test",
  "-p",
  "voxels-world-service",
  "--features",
  "automation-fixture",
]);
await cargo("workspace lints", [
  "clippy",
  "--workspace",
  "--exclude",
  "voxels-shell",
  "--all-targets",
  "--",
  "-D",
  "warnings",
]);
await cargo("world-service automation lints", [
  "clippy",
  "-p",
  "voxels-world-service",
  "--features",
  "automation-fixture",
  "--all-targets",
  "--",
  "-D",
  "warnings",
]);
await cargo(
  "WASM shell lints",
  [
    "clippy",
    "-p",
    "voxels-shell",
    "--target",
    "wasm32-unknown-unknown",
    "--all-targets",
    "--",
    "-D",
    "warnings",
  ],
  {
    ...process.env,
    ...wasmCcEnv(),
  },
);
if (process.platform === "darwin") {
  await cargo("Terrain Diffusion Metal tests", [
    "test",
    "-p",
    "voxels-world-terrain-diffusion",
    "--all-features",
  ]);
  await cargo("Terrain Diffusion Metal lints", [
    "clippy",
    "-p",
    "voxels-world-terrain-diffusion",
    "--all-features",
    "--all-targets",
    "--",
    "-D",
    "warnings",
  ]);
  await cargo("world-service Metal lints", [
    "clippy",
    "-p",
    "voxels-world-service",
    "--features",
    "terrain-metal",
    "--all-targets",
    "--",
    "-D",
    "warnings",
  ]);
}
