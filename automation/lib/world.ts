import { randomBytes } from "node:crypto";
import { execFile } from "node:child_process";
import type { ChildProcess } from "node:child_process";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import path from "node:path";
import { tmpdir } from "node:os";
import { connect } from "node:net";
import { promisify } from "node:util";
import { build, preview } from "vite-plus";
import type { PreviewServer } from "vite-plus";
import type { BrowserContext, Page } from "playwright";
import { reserveEphemeralPort } from "./browser.ts";
import { runProcess, startProcess } from "./process.ts";
import type { ScenarioContext } from "./scenario.ts";
import type { WasmBuildProfile } from "../../scripts/build-wasm.ts";
import { rustTool } from "../../scripts/build-wasm.ts";
import {
  worldServiceBuildCargoArgs,
  worldServiceExecutablePath,
} from "../../scripts/world-service-command.ts";
import type { WorldServiceCargoProfile } from "../../scripts/world-service-command.ts";

const execFileAsync = promisify(execFile);
const AUTOMATION_FIXTURE_SCHEMA_VERSION = 2;

export type WorldSource = "procedural-v16" | "terrain-diffusion-30m";

export interface WorldFixtureOptions {
  readonly originPort: number;
  readonly clientPorts?: readonly number[];
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

export interface WorldFixture {
  readonly directory: string;
  readonly backendPort: number;
  readonly originPort: number;
  readonly authToken: string;
  readonly clientConfigPath: string;
  readonly clientConfigPaths: readonly string[];
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

export interface StartWorldServiceOptions {
  readonly build?: boolean;
  readonly metal?: boolean;
  readonly profile?: WorldServiceCargoProfile;
}

export interface WorldService {
  readonly child: ChildProcess;
  readonly logs: string[];
  close(): Promise<void>;
}

export interface WebPreviewOptions {
  readonly port?: number;
  readonly build?: boolean;
  readonly buildProfile?: WasmBuildProfile;
}

export interface WebPreview {
  readonly port: number;
  readonly url: string;
  readonly server: PreviewServer;
}

export interface WorldClientRoute {
  readonly beforeNavigate: (context: BrowserContext, page: Page) => Promise<void>;
}

export interface WorldStackOptions {
  readonly fixture?: Omit<WorldFixtureOptions, "originPort">;
  readonly service?: StartWorldServiceOptions;
  readonly web?: Omit<WebPreviewOptions, "port">;
}

export interface WorldStack extends WebPreview {
  readonly fixture: WorldFixture;
  readonly service: WorldService;
  readonly clientRoute: WorldClientRoute;
}

type FixtureResolved = Omit<
  WorldFixture,
  | "directory"
  | "backendPort"
  | "originPort"
  | "authToken"
  | "clientConfigPath"
  | "clientConfigPaths"
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
  clientPorts: readonly number[],
  overlay: Readonly<Record<string, unknown>>,
): Promise<{
  readonly resolved: FixtureResolved;
  readonly clientConfigPaths: readonly string[];
}> {
  const requestPath = path.join(directory, "fixture-request.json");
  const responsePath = path.join(directory, "fixture-response.json");
  const serviceConfigPath = path.join(directory, "world-service.toml");
  const clientConfigPath = path.join(directory, "client.toml");
  const clientConfigPaths = clientPorts.map((_, index) =>
    path.join(directory, `client-${index + 1}.toml`),
  );
  await writeFile(
    requestPath,
    `${JSON.stringify(
      {
        serviceSourcePath: path.resolve("config/world-service.toml"),
        clientSourcePath: path.resolve("config/client.toml"),
        serviceOutputPath: serviceConfigPath,
        clientOutputPath: clientConfigPath,
        clientOutputPaths: clientConfigPaths,
        overlay: { ...overlay, clientPorts },
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
  return {
    resolved: parseFixtureResponse(JSON.parse(await readFile(responsePath, "utf8"))),
    clientConfigPaths: Object.freeze(clientConfigPaths),
  };
}

export async function prepareWorldFixture({
  originPort,
  clientPorts = [],
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
}: WorldFixtureOptions): Promise<WorldFixture> {
  const directory = await mkdtemp(path.join(tmpdir(), prefix));
  try {
    const backendPort = await reserveEphemeralPort();
    const authToken = randomBytes(32).toString("hex");
    const serviceConfigPath = path.join(directory, "world-service.toml");
    const clientConfigPath = path.join(directory, "client.toml");
    const generated = await writeTypedFixture(directory, clientPorts, {
      schemaVersion: AUTOMATION_FIXTURE_SCHEMA_VERSION,
      browserPort: originPort,
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

    let cleaned = false;
    return {
      directory,
      backendPort,
      originPort,
      authToken,
      clientConfigPath,
      clientConfigPaths: generated.clientConfigPaths,
      serviceConfigPath,
      databasePath: path.join(directory, "world-state.sqlite3"),
      ...generated.resolved,
      async cleanup() {
        if (cleaned) return;
        cleaned = true;
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

export async function startWorldService(
  context: ScenarioContext,
  fixture: WorldFixture,
  { build = true, metal = false, profile = "worldgen" }: StartWorldServiceOptions = {},
): Promise<WorldService> {
  if (build) {
    await runProcess(context, rustTool("cargo"), worldServiceBuildCargoArgs({ metal, profile }), {
      label: "world service build",
      cwd: process.cwd(),
      env: process.env,
      stdio: "inherit",
    });
  }
  const process_ = startProcess(
    context,
    worldServiceExecutablePath(profile),
    [fixture.serviceConfigPath],
    {
      label: "world service",
      cwd: process.cwd(),
      env: process.env,
      stdio: ["ignore", "pipe", "pipe"],
    },
  );
  const { child } = process_;
  const logs: string[] = [];
  for (const stream of [child.stdout, child.stderr]) {
    stream?.on("data", (bytes) => {
      logs.push(bytes.toString());
      if (logs.length > 200) logs.shift();
    });
  }
  void process_.completed.catch(() => {});
  const deadline = Date.now() + 300_000;
  while (Date.now() < deadline) {
    context.throwIfAborted();
    if (child.exitCode !== null || child.signalCode !== null) {
      throw new Error(`world service exited before readiness:\n${logs.join("")}`);
    }
    if (await portAcceptsConnections(fixture.backendPort)) {
      return { child, logs, close: () => process_.stop() };
    }
    await context.wait(75);
  }
  await process_.stop();
  throw new Error(`world service readiness timed out:\n${logs.join("")}`);
}

export function routeWorldClient(
  fixture: WorldFixture,
  routedClientIndex?: number,
): WorldClientRoute {
  const clientConfigPath =
    routedClientIndex === undefined
      ? fixture.clientConfigPath
      : fixture.clientConfigPaths[routedClientIndex];
  if (clientConfigPath === undefined) {
    throw new Error(`world fixture has no routed client config ${routedClientIndex}`);
  }
  return Object.freeze({
    async beforeNavigate(_context: BrowserContext, page: Page): Promise<void> {
      const clientConfig = await readFile(clientConfigPath, "utf8");
      await page.route("**/config/client.toml", (route) =>
        route.fulfill({
          status: 200,
          contentType: "text/plain; charset=utf-8",
          headers: { "Cache-Control": "no-store" },
          body: clientConfig,
        }),
      );
    },
  });
}

function automationBuildMode(profile: WasmBuildProfile): string {
  return `automation-${profile}`;
}

export async function startWebPreview(
  context: ScenarioContext,
  options: WebPreviewOptions = {},
): Promise<WebPreview> {
  const port = options.port ?? (await reserveEphemeralPort());
  const shouldBuild = options.build ?? true;
  context.throwIfAborted();
  if (shouldBuild) {
    await build({
      logLevel: "warn",
      ...(options.buildProfile === undefined
        ? {}
        : { mode: automationBuildMode(options.buildProfile) }),
    });
  }
  context.throwIfAborted();
  const server = await preview({
    logLevel: "warn",
    preview: { host: "127.0.0.1", port, strictPort: true },
  });
  context.defer("web preview", () => server.close());
  return { port, url: `http://127.0.0.1:${port}`, server };
}

export async function startWorldStack(
  context: ScenarioContext,
  options: WorldStackOptions = {},
): Promise<WorldStack> {
  if (!context.definition.uses.world || context.definition.uses.viewport !== "browser") {
    throw new Error(
      `scenario ${context.definition.id} must declare a browser viewport and world service`,
    );
  }
  const port = await reserveEphemeralPort();
  const fixture = await prepareWorldFixture({
    ...options.fixture,
    originPort: port,
  });
  context.defer("world fixture", () => fixture.cleanup());
  const web = await startWebPreview(context, {
    ...options.web,
    port,
  });
  const service = await startWorldService(context, fixture, {
    ...options.service,
    build: options.service?.build ?? options.web?.build ?? true,
  });
  return {
    ...web,
    fixture,
    service,
    clientRoute: routeWorldClient(fixture),
  };
}
