import benchPage from "./pages/bench.html";
import gamePage from "./pages/game.html";

const PROCEDURAL_WORKER_ROUTE = "/assets/procedural-generation-worker.js";
const DEVELOPMENT_WORKER_OUTDIR = `${process.cwd()}/.tmp/procedural-worker`;
let devWorkerAssetPathPromise: Promise<string> | null = null;

function isDevelopmentMode(): boolean {
  const nodeEnvKey = "NODE_ENV";
  return process.env[nodeEnvKey] !== "production";
}

async function buildDevelopmentWorkerAsset(): Promise<string> {
  const result = await Bun.build({
    entrypoints: ["./src/client/procedural-generation-worker.ts"],
    outdir: DEVELOPMENT_WORKER_OUTDIR,
    target: "browser",
    format: "esm",
    splitting: false,
    minify: false,
  });
  if (!result.success) {
    const messages = result.logs.map((log) => `[${log.level}] ${log.message}`).join("\n");
    throw new Error(`Development worker build failed:\n${messages}`);
  }
  const outputPath = result.outputs[0]?.path;
  if (!outputPath) {
    throw new Error("Development worker build did not emit an output file");
  }
  return outputPath;
}

async function serveProceduralWorker(): Promise<Response> {
  if (isDevelopmentMode()) {
    devWorkerAssetPathPromise ??= buildDevelopmentWorkerAsset();
    const filePath = await devWorkerAssetPathPromise;
    return new Response(Bun.file(filePath), {
      headers: {
        "Content-Type": "text/javascript; charset=utf-8",
        "Cache-Control": "no-store",
      },
    });
  }
  return new Response(Bun.file(new URL("./assets/procedural-generation-worker.js", import.meta.url)), {
    headers: {
      "Content-Type": "text/javascript; charset=utf-8",
      "Cache-Control": "public, max-age=31536000, immutable",
    },
  });
}

const server = Bun.serve({
  port: Number.parseInt(process.env.PORT ?? "3000", 10),
  development: isDevelopmentMode()
    ? {
        hmr: true,
        console: true,
      }
    : undefined,
  routes: {
    "/": gamePage,
    "/bench": benchPage,
    [PROCEDURAL_WORKER_ROUTE]: serveProceduralWorker,
    "/favicon.ico": () => new Response(null, { status: 204 }),
  },
  fetch() {
    return new Response("Not Found", { status: 404 });
  },
});

console.log(`Voxel server listening on http://localhost:${server.port}`);
