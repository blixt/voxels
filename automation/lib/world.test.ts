import { readFile } from "node:fs/promises";
import { describe, expect, it } from "vite-plus/test";
import { prepareWorldFixture } from "./world.ts";
import { PRESENCE_PATH, WORLD_PATH, WORLD_SUBPROTOCOL } from "./protocol.ts";

describe("isolated browser world fixture", () => {
  it("binds matching temporary configuration and preserves omitted renderer defaults", async () => {
    const environmentBefore = {
      client: process.env.VOXELS_CLIENT_CONFIG_PATH,
      service: process.env.VOXELS_WORLD_SERVICE_CONFIG_PATH,
      external: process.env.VOXELS_EXTERNAL_WORLD_SERVICE,
    };
    const fixture = await prepareWorldFixture({
      originPort: 41_234,
      spawnVoxels: [-12_800, 25_600],
      spawnPillarHeightVoxels: 7,
      spawnPillarRadiusVoxels: 2,
      spawnProtectionRadiusVoxels: 3,
      cascadedShadows: false,
      screenSpaceAmbientOcclusion: false,
      weatherCycleSeconds: 36,
      weatherFractionAtUnixEpoch: 0.62,
      cloudCoverage: 0.31,
      cloudBaseMetres: 600,
      cloudTopMetres: 1_400,
    });
    try {
      const [client, service] = await Promise.all([
        readFile(fixture.clientConfigPath, "utf8"),
        readFile(fixture.serviceConfigPath, "utf8"),
      ]);
      expect(client).toContain(`endpoint = "ws://127.0.0.1:${fixture.backendPort}${WORLD_PATH}"`);
      expect(client).toContain(
        `presence_endpoint = "ws://127.0.0.1:${fixture.backendPort}${PRESENCE_PATH}"`,
      );
      expect(client).toContain(`subprotocol = "${WORLD_SUBPROTOCOL}"`);
      expect(client).toContain(`auth_subprotocol_token = "${fixture.authToken}"`);
      expect(client).toContain("cascaded_sun_shadows = false");
      expect(client).toContain("screen_space_ambient_occlusion = false");
      expect(service).toContain('source = "procedural-v16"');
      expect(service).toContain(`listen = "127.0.0.1:${fixture.backendPort}"`);
      expect(service).toContain('allowed_origins = ["http://127.0.0.1:41234"]');
      expect(service).toContain(`auth_subprotocol_token = "${fixture.authToken}"`);
      expect(service).toContain('database = "world-state.sqlite3"');
      expect(service).toContain("pillar_height_voxels = 7");
      expect(service).toContain("pillar_radius_voxels = 2");
      expect(service).toContain("protection_radius_voxels = 3");
      expect(service).toContain("weather_cycle_seconds = 36");
      expect(service).toContain("weather_fraction_at_unix_epoch = 0.62");
      expect(service).toContain("cloud_coverage = 0.31");
      expect(service).toContain("cloud_base_metres = 600");
      expect(service).toContain("cloud_top_metres = 1400");
      expect(fixture.spawnVoxels).toEqual([-12_800, 25_600]);
      expect(fixture.spawnPillarHeightVoxels).toBe(7);
      expect(fixture.spawnPillarRadiusVoxels).toBe(2);
      expect(fixture.spawnProtectionRadiusVoxels).toBe(3);
      expect(fixture.cascadedShadows).toBe(false);
      expect(fixture.screenSpaceAmbientOcclusion).toBe(false);
      expect(fixture.weatherCycleSeconds).toBe(36);
      expect(fixture.weatherFractionAtUnixEpoch).toBe(0.62);
      expect(fixture.cloudCoverage).toBe(0.31);
      expect(fixture.databasePath.startsWith(fixture.directory)).toBe(true);
      expect({
        client: process.env.VOXELS_CLIENT_CONFIG_PATH,
        service: process.env.VOXELS_WORLD_SERVICE_CONFIG_PATH,
        external: process.env.VOXELS_EXTERNAL_WORLD_SERVICE,
      }).toEqual(environmentBefore);
    } finally {
      await fixture.cleanup();
    }

    const defaults = await prepareWorldFixture({ originPort: 41_235, clientPort: 41_236 });
    try {
      const client = await readFile(defaults.clientConfigPath, "utf8");
      expect(client).toContain(`endpoint = "ws://127.0.0.1:41236${WORLD_PATH}"`);
      expect(client).toContain(`presence_endpoint = "ws://127.0.0.1:41236${PRESENCE_PATH}"`);
      expect(client).toContain("cascaded_sun_shadows = true");
      expect(client).toContain("screen_space_ambient_occlusion = false");
      expect(defaults.cascadedShadows).toBe(true);
      expect(defaults.screenSpaceAmbientOcclusion).toBe(false);
    } finally {
      await defaults.cleanup();
    }
  }, 30_000);
});
