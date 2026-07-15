import { randomBytes } from "node:crypto";
import { spawn } from "node:child_process";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { connect } from "node:net";
import { reserveEphemeralPort } from "./browser-harness.mjs";
import { rustTool } from "./build-wasm.ts";
import { PRESENCE_PATH, WORLD_PATH, WORLD_SUBPROTOCOL } from "./vxwp-contract.mjs";
import { worldServiceCargoArgs } from "./world-service-command.ts";

function replaceEnvironment(name, value) {
  const previous = process.env[name];
  process.env[name] = value;
  return () => {
    if (previous === undefined) delete process.env[name];
    else process.env[name] = previous;
  };
}

export async function prepareBrowserWorldFixture({
  browserPort,
  prefix = "voxels-browser-world-",
  source = "procedural-v16",
}) {
  if (!Number.isInteger(browserPort) || browserPort <= 0 || browserPort > 65_535) {
    throw new Error("browser fixture port must be in 1..=65535");
  }
  const directory = await mkdtemp(path.join(tmpdir(), prefix));
  try {
    const backendPort = await reserveEphemeralPort();
    const authToken = randomBytes(32).toString("hex");
    const serviceConfigPath = path.join(directory, "world-service.toml");
    const clientConfigPath = path.join(directory, "client.toml");
    const [serviceSource, clientSource] = await Promise.all([
      readFile("config/world-service.toml", "utf8"),
      readFile("config/client.toml", "utf8"),
    ]);
    await Promise.all([
      writeFile(
        serviceConfigPath,
        serviceSource
          .replace(/^source = .*$/m, `source = "${source}"`)
          .replace(/^listen = .*$/m, `listen = "127.0.0.1:${backendPort}"`)
          .replace(
            /^allowed_origins = .*$/m,
            `allowed_origins = ["http://127.0.0.1:${browserPort}"]`,
          )
          .replace(/^auth_subprotocol_token = .*$/m, `auth_subprotocol_token = "${authToken}"`)
          .replace(/^database = .*$/m, 'database = "world-state.sqlite3"'),
      ),
      writeFile(
        clientConfigPath,
        clientSource
          .replace(/^endpoint = .*$/m, `endpoint = "ws://127.0.0.1:${backendPort}${WORLD_PATH}"`)
          .replace(
            /^presence_endpoint = .*$/m,
            `presence_endpoint = "ws://127.0.0.1:${backendPort}${PRESENCE_PATH}"`,
          )
          .replace(/^subprotocol = .*$/m, `subprotocol = "${WORLD_SUBPROTOCOL}"`)
          .replace(/^auth_subprotocol_token = .*$/m, `auth_subprotocol_token = "${authToken}"`),
      ),
    ]);

    const restoreClientConfig = replaceEnvironment("VOXELS_CLIENT_CONFIG_PATH", clientConfigPath);
    const restoreServiceConfig = replaceEnvironment(
      "VOXELS_WORLD_SERVICE_CONFIG_PATH",
      serviceConfigPath,
    );
    const restoreExternalService = replaceEnvironment("VOXELS_EXTERNAL_WORLD_SERVICE", "1");
    let cleaned = false;
    return {
      directory,
      backendPort,
      browserPort,
      authToken,
      clientConfigPath,
      serviceConfigPath,
      databasePath: path.join(directory, "world-state.sqlite3"),
      async cleanup() {
        if (cleaned) return;
        cleaned = true;
        restoreExternalService();
        restoreServiceConfig();
        restoreClientConfig();
        await rm(directory, { recursive: true, force: true });
      },
    };
  } catch (error) {
    await rm(directory, { recursive: true, force: true });
    throw error;
  }
}

function portAcceptsConnections(port) {
  return new Promise((resolve) => {
    const socket = connect({ host: "127.0.0.1", port });
    socket.setTimeout(250, () => {
      socket.destroy();
      resolve(false);
    });
    socket.once("connect", () => {
      socket.destroy();
      resolve(true);
    });
    socket.once("error", () => resolve(false));
  });
}

function signalProcessTree(child, signal) {
  if (child.exitCode !== null || child.signalCode !== null) return;
  try {
    if (process.platform !== "win32" && child.pid !== undefined) process.kill(-child.pid, signal);
    else child.kill(signal);
  } catch (error) {
    if (error.code !== "ESRCH") throw error;
  }
}

async function stopProcessTree(child) {
  if (child.exitCode !== null || child.signalCode !== null) return;
  const exited = new Promise((resolve) => child.once("exit", resolve));
  signalProcessTree(child, "SIGTERM");
  await Promise.race([exited, new Promise((resolve) => setTimeout(resolve, 2_000))]);
  if (child.exitCode === null && child.signalCode === null) {
    signalProcessTree(child, "SIGKILL");
    await exited;
  }
}

export async function startBrowserWorldService(fixture, { metal = false } = {}) {
  const child = spawn(
    rustTool("cargo"),
    worldServiceCargoArgs({ metal, configPath: fixture.serviceConfigPath }),
    {
      stdio: ["ignore", "pipe", "pipe"],
      detached: process.platform !== "win32",
    },
  );
  const logs = [];
  for (const stream of [child.stdout, child.stderr]) {
    stream.on("data", (bytes) => {
      logs.push(bytes.toString());
      if (logs.length > 200) logs.shift();
    });
  }
  const deadline = Date.now() + 300_000;
  while (Date.now() < deadline) {
    if (child.exitCode !== null || child.signalCode !== null) {
      throw new Error(`world service exited before readiness:\n${logs.join("")}`);
    }
    if (await portAcceptsConnections(fixture.backendPort)) {
      return { child, close: () => stopProcessTree(child) };
    }
    await new Promise((resolve) => setTimeout(resolve, 75));
  }
  await stopProcessTree(child);
  throw new Error(`world service readiness timed out:\n${logs.join("")}`);
}
