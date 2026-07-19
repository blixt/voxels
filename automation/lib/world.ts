import { randomBytes } from "node:crypto";
import { execFile, execFileSync, spawn } from "node:child_process";
import type { ChildProcess } from "node:child_process";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import path from "node:path";
import { tmpdir } from "node:os";
import { connect } from "node:net";
import { promisify } from "node:util";
import { build, preview } from "vite-plus";
import type { PreviewServer } from "vite-plus";
import { reserveEphemeralPort } from "./browser.ts";
import type { ScenarioContext } from "./scenario.ts";
import { rustTool } from "../../scripts/build-wasm.ts";
import {
  worldServiceBuildCargoArgs,
  worldServiceExecutablePath,
} from "../../scripts/world-service-command.ts";
import type { WorldServiceCargoProfile } from "../../scripts/world-service-command.ts";

const execFileAsync = promisify(execFile);
const AUTOMATION_FIXTURE_SCHEMA_VERSION = 1;

export type WorldSource = "procedural-v16" | "terrain-diffusion-30m";

export interface BrowserWorldFixtureOptions {
  readonly browserPort: number;
  readonly prefix?: string;
  readonly source?: WorldSource;
  readonly spawnVoxels?: readonly [number, number];
  readonly spawnPillarHeightVoxels?: number;
  readonly spawnPillarRadiusVoxels?: number;
  readonly spawnProtectionRadiusVoxels?: number;
  readonly cascadedShadows?: boolean;
  readonly screenSpaceAmbientOcclusion?: boolean;
  readonly dayLengthSeconds?: number;
  readonly worldDayNumberAtUnixEpoch?: number;
  readonly dayFractionAtUnixEpoch?: number;
  readonly daysPerYear?: number;
  readonly moonSiderealOrbitDays?: number;
  readonly moonOrbitPhaseAtWorldEpoch?: number;
  readonly planetCircumferenceMetres?: number;
  readonly axialTiltDegrees?: number;
  readonly moonOrbitInclinationDegrees?: number;
  readonly celestialSeed?: number;
  readonly celestialRevision?: number;
  readonly weatherCycleSeconds?: number;
  readonly weatherFractionAtUnixEpoch?: number;
  readonly cloudVelocityMetresPerSecond?: readonly [number, number];
  readonly cloudCoverage?: number;
  readonly cloudBaseMetres?: number;
  readonly cloudTopMetres?: number;
}

export interface BrowserWorldFixture {
  readonly directory: string;
  readonly backendPort: number;
  readonly browserPort: number;
  readonly authToken: string;
  readonly clientConfigPath: string;
  readonly serviceConfigPath: string;
  readonly databasePath: string;
  readonly spawnVoxels: readonly [number, number];
  readonly spawnPillarHeightVoxels: number;
  readonly spawnPillarRadiusVoxels: number;
  readonly spawnProtectionRadiusVoxels: number;
  readonly cascadedShadows: boolean;
  readonly screenSpaceAmbientOcclusion: boolean;
  readonly dayLengthSeconds: number;
  readonly worldDayNumberAtUnixEpoch: number;
  readonly dayFractionAtUnixEpoch: number;
  readonly daysPerYear: number;
  readonly moonSiderealOrbitDays: number;
  readonly moonOrbitPhaseAtWorldEpoch: number;
  readonly planetCircumferenceMetres: number;
  readonly axialTiltDegrees: number;
  readonly moonOrbitInclinationDegrees: number;
  readonly celestialSeed: number;
  readonly celestialRevision: number;
  readonly weatherCycleSeconds: number;
  readonly weatherFractionAtUnixEpoch: number;
  readonly cloudVelocityMetresPerSecond: readonly [number, number];
  readonly cloudCoverage: number;
  readonly cloudBaseMetres: number;
  readonly cloudTopMetres: number;
  cleanup(): Promise<void>;
}

export interface StartBrowserWorldServiceOptions {
  readonly build?: boolean;
  readonly metal?: boolean;
  readonly profile?: WorldServiceCargoProfile;
}

export interface BrowserWorldService {
  readonly child: ChildProcess;
  readonly logs: string[];
  close(): Promise<void>;
}

export interface WorldPreviewOptions {
  readonly fixture?: Omit<BrowserWorldFixtureOptions, "browserPort">;
  readonly service?: StartBrowserWorldServiceOptions;
  readonly build?: boolean;
}

export interface WorldPreview {
  readonly port: number;
  readonly url: string;
  readonly fixture: BrowserWorldFixture;
  readonly service: BrowserWorldService;
  readonly server: PreviewServer;
}

function replaceEnvironment(name: string, value: string): () => void {
  const previous = process.env[name];
  process.env[name] = value;
  return () => {
    if (previous === undefined) delete process.env[name];
    else process.env[name] = previous;
  };
}

type FixtureResolved = Omit<
  BrowserWorldFixture,
  | "directory"
  | "backendPort"
  | "browserPort"
  | "authToken"
  | "clientConfigPath"
  | "serviceConfigPath"
  | "databasePath"
  | "cleanup"
>;

const FIXTURE_NUMBER_FIELDS = [
  "spawnPillarHeightVoxels",
  "spawnPillarRadiusVoxels",
  "spawnProtectionRadiusVoxels",
  "dayLengthSeconds",
  "worldDayNumberAtUnixEpoch",
  "dayFractionAtUnixEpoch",
  "daysPerYear",
  "moonSiderealOrbitDays",
  "moonOrbitPhaseAtWorldEpoch",
  "planetCircumferenceMetres",
  "axialTiltDegrees",
  "moonOrbitInclinationDegrees",
  "celestialSeed",
  "celestialRevision",
  "weatherCycleSeconds",
  "weatherFractionAtUnixEpoch",
  "cloudCoverage",
  "cloudBaseMetres",
  "cloudTopMetres",
] as const satisfies readonly (keyof FixtureResolved)[];

const FIXTURE_BOOLEAN_FIELDS = [
  "cascadedShadows",
  "screenSpaceAmbientOcclusion",
] as const satisfies readonly (keyof FixtureResolved)[];

function record(value: unknown, label: string): Readonly<Record<string, unknown>> {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new Error(`${label} must be an object`);
  }
  return value as Readonly<Record<string, unknown>>;
}

function numberPair(value: unknown, label: string): readonly [number, number] {
  if (
    !Array.isArray(value) ||
    value.length !== 2 ||
    value.some((entry) => typeof entry !== "number" || !Number.isFinite(entry))
  ) {
    throw new Error(`${label} must contain two finite numbers`);
  }
  return [value[0] as number, value[1] as number];
}

function parseFixtureResponse(value: unknown): FixtureResolved {
  const response = record(value, "automation fixture response");
  if (response.schemaVersion !== AUTOMATION_FIXTURE_SCHEMA_VERSION) {
    throw new Error(
      `automation fixture schema ${String(response.schemaVersion)} does not match ${AUTOMATION_FIXTURE_SCHEMA_VERSION}`,
    );
  }
  const resolved = record(response.resolved, "automation fixture resolved values");
  for (const field of FIXTURE_NUMBER_FIELDS) {
    if (typeof resolved[field] !== "number" || !Number.isFinite(resolved[field])) {
      throw new Error(`automation fixture omitted numeric ${field}`);
    }
  }
  for (const field of FIXTURE_BOOLEAN_FIELDS) {
    if (typeof resolved[field] !== "boolean") {
      throw new Error(`automation fixture omitted boolean ${field}`);
    }
  }
  return Object.freeze({
    ...resolved,
    spawnVoxels: numberPair(resolved.spawnVoxels, "automation fixture spawnVoxels"),
    cloudVelocityMetresPerSecond: numberPair(
      resolved.cloudVelocityMetresPerSecond,
      "automation fixture cloudVelocityMetresPerSecond",
    ),
  }) as FixtureResolved;
}

async function writeTypedFixture(
  directory: string,
  overlay: Readonly<Record<string, unknown>>,
): Promise<FixtureResolved> {
  const requestPath = path.join(directory, "fixture-request.json");
  const responsePath = path.join(directory, "fixture-response.json");
  const serviceConfigPath = path.join(directory, "world-service.toml");
  const clientConfigPath = path.join(directory, "client.toml");
  await writeFile(
    requestPath,
    `${JSON.stringify(
      {
        serviceSourcePath: path.resolve("config/world-service.toml"),
        clientSourcePath: path.resolve("config/client.toml"),
        serviceOutputPath: serviceConfigPath,
        clientOutputPath: clientConfigPath,
        overlay,
      },
      null,
      2,
    )}\n`,
  );
  await execFileAsync(
    rustTool("cargo"),
    [
      "run",
      "--quiet",
      "--profile",
      "worldgen-dev",
      "-p",
      "voxels-world-service",
      "--features",
      "automation-fixture",
      "--bin",
      "voxels-automation-fixture",
      "--",
      requestPath,
      responsePath,
    ],
    { cwd: process.cwd(), maxBuffer: 16 * 1024 * 1024 },
  );
  return parseFixtureResponse(JSON.parse(await readFile(responsePath, "utf8")));
}

export async function prepareBrowserWorldFixture({
  browserPort,
  prefix = "voxels-browser-world-",
  source = "procedural-v16",
  spawnVoxels,
  spawnPillarHeightVoxels,
  spawnPillarRadiusVoxels,
  spawnProtectionRadiusVoxels,
  cascadedShadows,
  screenSpaceAmbientOcclusion,
  dayLengthSeconds,
  worldDayNumberAtUnixEpoch,
  dayFractionAtUnixEpoch,
  daysPerYear,
  moonSiderealOrbitDays,
  moonOrbitPhaseAtWorldEpoch,
  planetCircumferenceMetres,
  axialTiltDegrees,
  moonOrbitInclinationDegrees,
  celestialSeed,
  celestialRevision,
  weatherCycleSeconds,
  weatherFractionAtUnixEpoch,
  cloudVelocityMetresPerSecond,
  cloudCoverage,
  cloudBaseMetres,
  cloudTopMetres,
}: BrowserWorldFixtureOptions): Promise<BrowserWorldFixture> {
  const directory = await mkdtemp(path.join(tmpdir(), prefix));
  try {
    const backendPort = await reserveEphemeralPort();
    const authToken = randomBytes(32).toString("hex");
    const serviceConfigPath = path.join(directory, "world-service.toml");
    const clientConfigPath = path.join(directory, "client.toml");
    const resolved = await writeTypedFixture(directory, {
      schemaVersion: AUTOMATION_FIXTURE_SCHEMA_VERSION,
      browserPort,
      backendPort,
      authToken,
      source,
      spawnVoxels,
      spawnPillarHeightVoxels,
      spawnPillarRadiusVoxels,
      spawnProtectionRadiusVoxels,
      cascadedShadows,
      screenSpaceAmbientOcclusion,
      dayLengthSeconds,
      worldDayNumberAtUnixEpoch,
      dayFractionAtUnixEpoch,
      daysPerYear,
      moonSiderealOrbitDays,
      moonOrbitPhaseAtWorldEpoch,
      planetCircumferenceMetres,
      axialTiltDegrees,
      moonOrbitInclinationDegrees,
      celestialSeed,
      celestialRevision,
      weatherCycleSeconds,
      weatherFractionAtUnixEpoch,
      cloudVelocityMetresPerSecond,
      cloudCoverage,
      cloudBaseMetres,
      cloudTopMetres,
    });

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
      ...resolved,
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

function portAcceptsConnections(port: number): Promise<boolean> {
  return new Promise<boolean>((resolve) => {
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

function signalProcessTree(child: ChildProcess, signal: NodeJS.Signals): void {
  if (child.exitCode !== null || child.signalCode !== null) return;
  try {
    if (process.platform !== "win32" && child.pid !== undefined) process.kill(-child.pid, signal);
    else child.kill(signal);
  } catch (error) {
    if (!(error instanceof Error && "code" in error && error.code === "ESRCH")) throw error;
  }
}

async function stopProcessTree(child: ChildProcess): Promise<void> {
  if (child.exitCode !== null || child.signalCode !== null) return;
  const exited = new Promise<void>((resolve) => {
    child.once("exit", () => resolve());
  });
  signalProcessTree(child, "SIGTERM");
  await Promise.race([exited, new Promise((resolve) => setTimeout(resolve, 2_000))]);
  if (child.exitCode === null && child.signalCode === null) {
    signalProcessTree(child, "SIGKILL");
    await exited;
  }
}

export async function startBrowserWorldService(
  fixture: BrowserWorldFixture,
  { build = true, metal = false, profile = "worldgen" }: StartBrowserWorldServiceOptions = {},
): Promise<BrowserWorldService> {
  if (build) {
    execFileSync(rustTool("cargo"), worldServiceBuildCargoArgs({ metal, profile }), {
      cwd: process.cwd(),
      env: process.env,
      stdio: "inherit",
    });
  }
  const child = spawn(worldServiceExecutablePath(profile), [fixture.serviceConfigPath], {
    cwd: process.cwd(),
    env: process.env,
    stdio: ["ignore", "pipe", "pipe"],
    detached: process.platform !== "win32",
  });
  const logs: string[] = [];
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
      return { child, logs, close: () => stopProcessTree(child) };
    }
    await new Promise((resolve) => setTimeout(resolve, 75));
  }
  await stopProcessTree(child);
  throw new Error(`world service readiness timed out:\n${logs.join("")}`);
}

export async function startWorldPreview(
  context: ScenarioContext,
  options: WorldPreviewOptions = {},
): Promise<WorldPreview> {
  if (!context.definition.uses.world || context.definition.uses.viewport !== "browser") {
    throw new Error(
      `scenario ${context.definition.id} must declare a browser viewport and world service`,
    );
  }
  const port = await reserveEphemeralPort();
  const fixture = await prepareBrowserWorldFixture({
    ...options.fixture,
    browserPort: port,
  });
  context.defer("world fixture", () => fixture.cleanup());
  if (options.build ?? true) await build({ logLevel: "warn" });
  const service = await startBrowserWorldService(fixture, {
    ...options.service,
    build: options.service?.build ?? options.build ?? true,
  });
  context.defer("world service", () => service.close());
  const server = await preview({
    logLevel: "warn",
    preview: { host: "127.0.0.1", port, strictPort: true },
  });
  context.defer("world preview", () => server.close());
  return {
    port,
    url: `http://127.0.0.1:${port}`,
    fixture,
    service,
    server,
  };
}
