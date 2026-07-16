import { readFile } from "node:fs/promises";
import { describe, expect, it } from "vite-plus/test";
import { prepareBrowserWorldFixture } from "./browser-world-fixture.mjs";
import { PRESENCE_PATH, WORLD_PATH, WORLD_SUBPROTOCOL } from "./vxwp-contract.mjs";

describe("isolated browser world fixture", () => {
  it("binds matching temporary client and procedural server configuration", async () => {
    const previousClient = process.env.VOXELS_CLIENT_CONFIG_PATH;
    const previousService = process.env.VOXELS_WORLD_SERVICE_CONFIG_PATH;
    const previousExternalService = process.env.VOXELS_EXTERNAL_WORLD_SERVICE;
    const fixture = await prepareBrowserWorldFixture({
      browserPort: 41_234,
      spawnVoxels: [-12_800, 25_600],
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
      expect(service).toContain("xz_voxels = [-12800, 25600]");
      expect(service).toContain("weather_cycle_seconds = 36");
      expect(service).toContain("weather_fraction_at_unix_epoch = 0.62");
      expect(service).toContain("cloud_coverage = 0.31");
      expect(service).toContain("cloud_base_metres = 600");
      expect(service).toContain("cloud_top_metres = 1400");
      expect(fixture.spawnVoxels).toEqual([-12_800, 25_600]);
      expect(fixture.cascadedShadows).toBe(false);
      expect(fixture.screenSpaceAmbientOcclusion).toBe(false);
      expect(fixture.weatherCycleSeconds).toBe(36);
      expect(fixture.weatherFractionAtUnixEpoch).toBe(0.62);
      expect(fixture.cloudCoverage).toBe(0.31);
      expect(fixture.databasePath.startsWith(fixture.directory)).toBe(true);
      expect(process.env.VOXELS_CLIENT_CONFIG_PATH).toBe(fixture.clientConfigPath);
      expect(process.env.VOXELS_WORLD_SERVICE_CONFIG_PATH).toBe(fixture.serviceConfigPath);
      expect(process.env.VOXELS_EXTERNAL_WORLD_SERVICE).toBe("1");
    } finally {
      await fixture.cleanup();
    }
    expect(process.env.VOXELS_CLIENT_CONFIG_PATH).toBe(previousClient);
    expect(process.env.VOXELS_WORLD_SERVICE_CONFIG_PATH).toBe(previousService);
    expect(process.env.VOXELS_EXTERNAL_WORLD_SERVICE).toBe(previousExternalService);
  });
});
