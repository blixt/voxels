const entrypoints = ["src/client/playground.ts", "src/client/bench.ts"];

export async function buildClientBundles(): Promise<void> {
  const result = await Bun.build({
    entrypoints,
    outdir: "public/build",
    minify: process.env.NODE_ENV === "production",
    sourcemap: "linked",
    target: "browser",
    splitting: false,
  });
  if (!result.success) {
    const messages = result.logs.map((log) => `[${log.level}] ${log.message}`).join("\n");
    throw new Error(`Client build failed:\n${messages}`);
  }
}
