import { readFileSync } from "node:fs";
import { describe, expect, it } from "vite-plus/test";
import { PRESENCE_PATH, VXWP_VERSION, WORLD_PATH, WORLD_SUBPROTOCOL } from "./protocol.ts";

function requiredMatch(source: string, pattern: RegExp, description: string): string {
  const value = pattern.exec(source)?.[1];
  if (!value) throw new Error(`could not read ${description}`);
  return value;
}

describe("VXWP script contract", () => {
  it("matches the Rust protocol, server routes, and checked-in client configs", () => {
    const protocolSource = readFileSync("world/src/protocol.rs", "utf8");
    const terrainPageSource = readFileSync("world/src/terrain_page.rs", "utf8");
    const serverSource = readFileSync("world-service/src/server.rs", "utf8");
    const clientConfigs = ["config/client.toml", "config/client.production.toml"].map((path) => ({
      path,
      source: readFileSync(path, "utf8"),
    }));
    const streamingDocs = readFileSync("docs/native-world-streaming.md", "utf8");

    expect(
      Number(
        requiredMatch(
          protocolSource,
          /pub const PROTOCOL_VERSION: u16 = (\d+);/u,
          "Rust protocol version",
        ),
      ),
    ).toBe(VXWP_VERSION);
    expect(
      requiredMatch(
        serverSource,
        /pub const WORLD_WEBSOCKET_PATH: &str = "([^"]+)";/u,
        "world WebSocket path",
      ),
    ).toBe(WORLD_PATH);
    expect(
      requiredMatch(
        serverSource,
        /pub const PRESENCE_WEBSOCKET_PATH: &str = "([^"]+)";/u,
        "presence WebSocket path",
      ),
    ).toBe(PRESENCE_PATH);
    expect(
      requiredMatch(
        serverSource,
        /pub const WORLD_WEBSOCKET_PROTOCOL: &str = "([^"]+)";/u,
        "world WebSocket subprotocol",
      ),
    ).toBe(WORLD_SUBPROTOCOL);
    for (const config of clientConfigs) {
      expect(
        new URL(
          requiredMatch(config.source, /^endpoint = "([^"]+)"$/mu, `${config.path} world endpoint`),
        ).pathname,
      ).toBe(WORLD_PATH);
      expect(
        new URL(
          requiredMatch(
            config.source,
            /^presence_endpoint = "([^"]+)"$/mu,
            `${config.path} presence endpoint`,
          ),
        ).pathname,
      ).toBe(PRESENCE_PATH);
      expect(
        requiredMatch(config.source, /^subprotocol = "([^"]+)"$/mu, `${config.path} subprotocol`),
      ).toBe(WORLD_SUBPROTOCOL);
    }
    expect(
      Number(
        requiredMatch(
          streamingDocs,
          /Further `VXTP` v(\d+) pages/u,
          "documented terrain page schema version",
        ),
      ),
    ).toBe(
      Number(
        requiredMatch(
          terrainPageSource,
          /pub const TERRAIN_PAGE_SCHEMA_VERSION: u16 = (\d+);/u,
          "Rust terrain page schema version",
        ),
      ),
    );
  });
});
