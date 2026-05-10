export {};

await Bun.$`rm -rf dist`;

process.env.NODE_ENV = "production";

const result = await Bun.build({
  entrypoints: ["./src/server.ts"],
  outdir: "./dist",
  target: "bun",
  minify: true,
  sourcemap: "linked",
});

if (!result.success) {
  const messages = result.logs.map((log) => `[${log.level}] ${log.message}`).join("\n");
  throw new Error(`Production build failed:\n${messages}`);
}

const workerResult = await Bun.build({
  entrypoints: [
    "./src/client/procedural-generation-worker.ts",
    "./src/client/chunk-meshing-worker.ts",
  ],
  outdir: "./dist/assets",
  target: "browser",
  format: "esm",
  splitting: false,
  minify: true,
});

if (!workerResult.success) {
  const messages = workerResult.logs.map((log) => `[${log.level}] ${log.message}`).join("\n");
  throw new Error(`Production worker build failed:\n${messages}`);
}

await Bun.$`cp public/styles.css dist/styles.css`;

console.log("Built production server bundle into dist/");
