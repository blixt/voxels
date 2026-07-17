import { randomBytes } from "node:crypto";
import { execFileSync, spawn } from "node:child_process";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import path from "node:path";
import { tmpdir } from "node:os";
import { connect } from "node:net";
import { reserveEphemeralPort } from "./browser-harness.mjs";
import { rustTool } from "./build-wasm.ts";
import { PRESENCE_PATH, WORLD_PATH, WORLD_SUBPROTOCOL } from "./vxwp-contract.mjs";
import { worldServiceBuildCargoArgs, worldServiceExecutablePath } from "./world-service-command.ts";

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
  spawnVoxels,
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
}) {
  if (!Number.isInteger(browserPort) || browserPort <= 0 || browserPort > 65_535) {
    throw new Error("browser fixture port must be in 1..=65535");
  }
  if (
    spawnVoxels !== undefined &&
    (!Array.isArray(spawnVoxels) ||
      spawnVoxels.length !== 2 ||
      !spawnVoxels.every(
        (value) => Number.isInteger(value) && value >= -2_147_483_648 && value <= 2_147_483_647,
      ))
  ) {
    throw new Error("browser fixture spawnVoxels must contain two signed 32-bit integers");
  }
  if (cascadedShadows !== undefined && typeof cascadedShadows !== "boolean") {
    throw new Error("browser fixture cascadedShadows must be boolean when provided");
  }
  if (
    screenSpaceAmbientOcclusion !== undefined &&
    typeof screenSpaceAmbientOcclusion !== "boolean"
  ) {
    throw new Error("browser fixture screenSpaceAmbientOcclusion must be boolean when provided");
  }
  if (
    dayLengthSeconds !== undefined &&
    (!Number.isFinite(dayLengthSeconds) || dayLengthSeconds < 0 || dayLengthSeconds > 86_400)
  ) {
    throw new Error("browser fixture dayLengthSeconds must be finite and in 0..=86400");
  }
  if (
    worldDayNumberAtUnixEpoch !== undefined &&
    (!Number.isSafeInteger(worldDayNumberAtUnixEpoch) ||
      Math.abs(worldDayNumberAtUnixEpoch) > 1_000_000_000)
  ) {
    throw new Error("browser fixture world day number must be a safe integer in +/-1e9");
  }
  if (
    dayFractionAtUnixEpoch !== undefined &&
    (!Number.isFinite(dayFractionAtUnixEpoch) ||
      dayFractionAtUnixEpoch < 0 ||
      dayFractionAtUnixEpoch >= 1)
  ) {
    throw new Error("browser fixture dayFractionAtUnixEpoch must be finite and in 0..<1");
  }
  if (
    daysPerYear !== undefined &&
    (!Number.isFinite(daysPerYear) || daysPerYear < 4 || daysPerYear > 4_096)
  ) {
    throw new Error("browser fixture daysPerYear must be finite and in 4..=4096");
  }
  const resolvedDaysPerYear = daysPerYear ?? 365.2422;
  if (
    moonSiderealOrbitDays !== undefined &&
    (!Number.isFinite(moonSiderealOrbitDays) ||
      moonSiderealOrbitDays < 0.25 ||
      moonSiderealOrbitDays > resolvedDaysPerYear)
  ) {
    throw new Error("browser fixture lunar orbit must be in 0.25..=daysPerYear");
  }
  if (
    moonOrbitPhaseAtWorldEpoch !== undefined &&
    (!Number.isFinite(moonOrbitPhaseAtWorldEpoch) ||
      moonOrbitPhaseAtWorldEpoch < 0 ||
      moonOrbitPhaseAtWorldEpoch >= 1)
  ) {
    throw new Error("browser fixture lunar epoch phase must be finite and in 0..<1");
  }
  if (
    planetCircumferenceMetres !== undefined &&
    (!Number.isFinite(planetCircumferenceMetres) ||
      planetCircumferenceMetres < 100_000 ||
      planetCircumferenceMetres > 100_000_000)
  ) {
    throw new Error("browser fixture planet circumference must be in 100km..=100000km");
  }
  if (
    axialTiltDegrees !== undefined &&
    (!Number.isFinite(axialTiltDegrees) || axialTiltDegrees < 0 || axialTiltDegrees > 45)
  ) {
    throw new Error("browser fixture axial tilt must be finite and in 0..=45 degrees");
  }
  if (
    moonOrbitInclinationDegrees !== undefined &&
    (!Number.isFinite(moonOrbitInclinationDegrees) ||
      moonOrbitInclinationDegrees < 0 ||
      moonOrbitInclinationDegrees > 30)
  ) {
    throw new Error("browser fixture lunar inclination must be finite and in 0..=30 degrees");
  }
  for (const [name, value] of [
    ["celestialSeed", celestialSeed],
    ["celestialRevision", celestialRevision],
  ]) {
    if (value !== undefined && (!Number.isSafeInteger(value) || value < 1)) {
      throw new Error(`browser fixture ${name} must be a positive safe integer`);
    }
  }
  if (
    cloudVelocityMetresPerSecond !== undefined &&
    (!Array.isArray(cloudVelocityMetresPerSecond) ||
      cloudVelocityMetresPerSecond.length !== 2 ||
      !cloudVelocityMetresPerSecond.every(
        (value) => Number.isFinite(value) && Math.abs(value) <= 100,
      ))
  ) {
    throw new Error("browser fixture cloud velocity must contain two values in -100..=100");
  }
  if (
    weatherCycleSeconds !== undefined &&
    (!Number.isFinite(weatherCycleSeconds) ||
      weatherCycleSeconds < 0 ||
      weatherCycleSeconds > 86_400)
  ) {
    throw new Error("browser fixture weatherCycleSeconds must be finite and in 0..=86400");
  }
  if (
    weatherFractionAtUnixEpoch !== undefined &&
    (!Number.isFinite(weatherFractionAtUnixEpoch) ||
      weatherFractionAtUnixEpoch < 0 ||
      weatherFractionAtUnixEpoch >= 1)
  ) {
    throw new Error("browser fixture weatherFractionAtUnixEpoch must be finite and in 0..<1");
  }
  if (
    cloudCoverage !== undefined &&
    (!Number.isFinite(cloudCoverage) || cloudCoverage < 0 || cloudCoverage > 1)
  ) {
    throw new Error("browser fixture cloudCoverage must be finite and in 0..=1");
  }
  const resolvedCloudBaseMetres = cloudBaseMetres ?? 550;
  const resolvedCloudTopMetres = cloudTopMetres ?? 1_800;
  if (
    !Number.isFinite(resolvedCloudBaseMetres) ||
    resolvedCloudBaseMetres < 100 ||
    resolvedCloudBaseMetres > 5_000 ||
    !Number.isFinite(resolvedCloudTopMetres) ||
    resolvedCloudTopMetres <= resolvedCloudBaseMetres ||
    resolvedCloudTopMetres > 10_000
  ) {
    throw new Error("browser fixture cloud layer must have 100..=5000m base below a <=10000m top");
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
          .replace(/^database = .*$/m, 'database = "world-state.sqlite3"')
          .replace(
            /^day_length_seconds = .*$/m,
            `day_length_seconds = ${dayLengthSeconds ?? 1_200}`,
          )
          .replace(
            /^world_day_number_at_unix_epoch = .*$/m,
            `world_day_number_at_unix_epoch = ${worldDayNumberAtUnixEpoch ?? 0}`,
          )
          .replace(
            /^day_fraction_at_unix_epoch = .*$/m,
            `day_fraction_at_unix_epoch = ${dayFractionAtUnixEpoch ?? 0.72}`,
          )
          .replace(/^days_per_year = .*$/m, `days_per_year = ${resolvedDaysPerYear}`)
          .replace(
            /^moon_sidereal_orbit_days = .*$/m,
            `moon_sidereal_orbit_days = ${moonSiderealOrbitDays ?? 27.321661}`,
          )
          .replace(
            /^moon_orbit_phase_at_world_epoch = .*$/m,
            `moon_orbit_phase_at_world_epoch = ${moonOrbitPhaseAtWorldEpoch ?? 0}`,
          )
          .replace(
            /^planet_circumference_metres = .*$/m,
            `planet_circumference_metres = ${planetCircumferenceMetres ?? 40_075_016}`,
          )
          .replace(
            /^axial_tilt_degrees = .*$/m,
            `axial_tilt_degrees = ${axialTiltDegrees ?? 23.4393}`,
          )
          .replace(
            /^moon_orbit_inclination_degrees = .*$/m,
            `moon_orbit_inclination_degrees = ${moonOrbitInclinationDegrees ?? 5.145}`,
          )
          .replace(/^celestial_seed = .*$/m, `celestial_seed = ${celestialSeed ?? 1_470_258_925}`)
          .replace(/^celestial_revision = .*$/m, `celestial_revision = ${celestialRevision ?? 1}`)
          .replace(
            /^weather_cycle_seconds = .*$/m,
            `weather_cycle_seconds = ${weatherCycleSeconds ?? 900}`,
          )
          .replace(
            /^weather_fraction_at_unix_epoch = .*$/m,
            `weather_fraction_at_unix_epoch = ${weatherFractionAtUnixEpoch ?? 0.08}`,
          )
          .replace(
            /^cloud_velocity_metres_per_second = .*$/m,
            `cloud_velocity_metres_per_second = [${(cloudVelocityMetresPerSecond ?? [5.5, 1.6]).join(", ")}]`,
          )
          .replace(/^cloud_coverage = .*$/m, `cloud_coverage = ${cloudCoverage ?? 0.24}`)
          .replace(/^cloud_base_metres = .*$/m, `cloud_base_metres = ${resolvedCloudBaseMetres}`)
          .replace(/^cloud_top_metres = .*$/m, `cloud_top_metres = ${resolvedCloudTopMetres}`)
          .replace(
            /^xz_voxels = .*$/m,
            `xz_voxels = [${spawnVoxels?.[0] ?? 0}, ${spawnVoxels?.[1] ?? 0}]`,
          ),
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
          .replace(/^auth_subprotocol_token = .*$/m, `auth_subprotocol_token = "${authToken}"`)
          .replace(
            /^cascaded_sun_shadows = .*$/m,
            `cascaded_sun_shadows = ${cascadedShadows ?? true}`,
          )
          .replace(
            /^screen_space_ambient_occlusion = .*$/m,
            `screen_space_ambient_occlusion = ${screenSpaceAmbientOcclusion ?? true}`,
          ),
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
      spawnVoxels: spawnVoxels ?? [0, 0],
      cascadedShadows: cascadedShadows ?? true,
      screenSpaceAmbientOcclusion: screenSpaceAmbientOcclusion ?? true,
      dayLengthSeconds: dayLengthSeconds ?? 1_200,
      worldDayNumberAtUnixEpoch: worldDayNumberAtUnixEpoch ?? 0,
      dayFractionAtUnixEpoch: dayFractionAtUnixEpoch ?? 0.72,
      daysPerYear: resolvedDaysPerYear,
      moonSiderealOrbitDays: moonSiderealOrbitDays ?? 27.321661,
      moonOrbitPhaseAtWorldEpoch: moonOrbitPhaseAtWorldEpoch ?? 0,
      planetCircumferenceMetres: planetCircumferenceMetres ?? 40_075_016,
      axialTiltDegrees: axialTiltDegrees ?? 23.4393,
      moonOrbitInclinationDegrees: moonOrbitInclinationDegrees ?? 5.145,
      celestialSeed: celestialSeed ?? 1_470_258_925,
      celestialRevision: celestialRevision ?? 1,
      weatherCycleSeconds: weatherCycleSeconds ?? 900,
      weatherFractionAtUnixEpoch: weatherFractionAtUnixEpoch ?? 0.08,
      cloudVelocityMetresPerSecond: cloudVelocityMetresPerSecond ?? [5.5, 1.6],
      cloudCoverage: cloudCoverage ?? 0.24,
      cloudBaseMetres: resolvedCloudBaseMetres,
      cloudTopMetres: resolvedCloudTopMetres,
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

export async function startBrowserWorldService(
  fixture,
  { build = true, metal = false, profile = "worldgen" } = {},
) {
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
