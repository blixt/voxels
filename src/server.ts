import { buildClientBundles } from "./server/build-client.ts";
import { renderBenchPage, renderPlaygroundPage } from "./server/templates.ts";

await buildClientBundles();

const NO_STORE_HEADERS = {
  "cache-control": "no-store, max-age=0",
  pragma: "no-cache",
  expires: "0",
};

function assetVersion(): string {
  return Date.now().toString(36);
}

function htmlResponse(markup: string): Response {
  return new Response(markup, {
    headers: {
      "content-type": "text/html; charset=utf-8",
      ...NO_STORE_HEADERS,
    },
  });
}

const server = Bun.serve({
  port: Number.parseInt(process.env.PORT ?? "3000", 10),
  routes: {
    "/": () => htmlResponse(renderPlaygroundPage(assetVersion())),
    "/bench": () => htmlResponse(renderBenchPage(assetVersion())),
    "/styles.css": () => new Response(Bun.file("public/styles.css"), { headers: NO_STORE_HEADERS }),
    "/favicon.ico": () => new Response(null, { status: 204 }),
    "/build/:file": (request) => {
      const url = new URL(request.url);
      const file = url.pathname.replace("/build/", "");
      return new Response(Bun.file(`public/build/${file}`), { headers: NO_STORE_HEADERS });
    },
  },
  fetch() {
    return new Response("Not Found", { status: 404 });
  },
});

console.log(`Voxel server listening on http://localhost:${server.port}`);
