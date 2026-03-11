import benchPage from "./pages/bench.html";
import gamePage from "./pages/game.html";

function isDevelopmentMode(): boolean {
  const nodeEnvKey = "NODE_ENV";
  return process.env[nodeEnvKey] !== "production";
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
    "/favicon.ico": () => new Response(null, { status: 204 }),
  },
  fetch() {
    return new Response("Not Found", { status: 404 });
  },
});

console.log(`Voxel server listening on http://localhost:${server.port}`);
