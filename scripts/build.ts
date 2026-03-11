export {};

await Bun.$`rm -rf dist`;

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

console.log("Built production server bundle into dist/");
