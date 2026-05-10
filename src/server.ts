import benchPage from "./pages/bench.html";
import gamePage from "./pages/game.html";
import lodLabPage from "./pages/lod-lab.html";

const PROCEDURAL_WORKER_ROUTE = "/assets/procedural-generation-worker.js";
const CHUNK_MESHING_WORKER_ROUTE = "/assets/chunk-meshing-worker.js";
const STYLESHEET_ROUTE = "/styles.css";
const DEVELOPMENT_WORKER_OUTDIR = `${process.cwd()}/.tmp/worker-assets`;
const devWorkerAssetPathPromises = new Map<string, Promise<string>>();

function isDevelopmentMode(): boolean {
  const env = Reflect.get(process, "env") as Record<string, string | undefined>;
  return env.NODE_ENV !== "production";
}

async function buildDevelopmentWorkerAsset(entrypoint: string, assetName: string): Promise<string> {
  const result = await Bun.build({
    entrypoints: [entrypoint],
    outdir: `${DEVELOPMENT_WORKER_OUTDIR}/${assetName}`,
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

async function serveWorkerAsset(routePath: string, entrypoint: string, assetFileName: string): Promise<Response> {
  if (isDevelopmentMode()) {
    let assetPathPromise = devWorkerAssetPathPromises.get(routePath);
    if (!assetPathPromise) {
      assetPathPromise = buildDevelopmentWorkerAsset(entrypoint, assetFileName);
      devWorkerAssetPathPromises.set(routePath, assetPathPromise);
    }
    const filePath = await assetPathPromise;
    return new Response(Bun.file(filePath), {
      headers: {
        "Content-Type": "text/javascript; charset=utf-8",
        "Cache-Control": "no-store",
      },
    });
  }
  return new Response(Bun.file(new URL(`./assets/${assetFileName}`, import.meta.url)), {
    headers: {
      "Content-Type": "text/javascript; charset=utf-8",
      "Cache-Control": "public, max-age=31536000, immutable",
    },
  });
}

function serveStylesheet(): Response {
  const stylesheetUrl = isDevelopmentMode()
    ? new URL("../public/styles.css", import.meta.url)
    : new URL("./styles.css", import.meta.url);
  return new Response(Bun.file(stylesheetUrl), {
    headers: {
      "Content-Type": "text/css; charset=utf-8",
      "Cache-Control": isDevelopmentMode()
        ? "no-store"
        : "public, max-age=31536000, immutable",
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
    "/lod-lab": lodLabPage,
    [STYLESHEET_ROUTE]: serveStylesheet,
    [PROCEDURAL_WORKER_ROUTE]: () =>
      serveWorkerAsset(PROCEDURAL_WORKER_ROUTE, "./src/client/procedural-generation-worker.ts", "procedural-generation-worker.js"),
    [CHUNK_MESHING_WORKER_ROUTE]: () =>
      serveWorkerAsset(CHUNK_MESHING_WORKER_ROUTE, "./src/client/chunk-meshing-worker.ts", "chunk-meshing-worker.js"),
    "/favicon.ico": () => new Response(null, { status: 204 }),
  },
  fetch() {
    return new Response("Not Found", { status: 404 });
  },
});

console.log(`Voxel server listening on http://localhost:${server.port}`);
