import { readFileSync } from "node:fs";
import { describe, expect, it } from "vite-plus/test";
import { PRESENCE_PATH, VXWP_VERSION, WORLD_PATH, WORLD_SUBPROTOCOL } from "./vxwp-contract.mjs";

function requiredMatch(source, pattern, description) {
  const value = pattern.exec(source)?.[1];
  if (!value) throw new Error(`could not read ${description}`);
  return value;
}

describe("VXWP script contract", () => {
  it("matches the Rust protocol, server routes, and checked-in client config", () => {
    const protocolSource = readFileSync("world/src/protocol.rs", "utf8");
    const serverSource = readFileSync("world-service/src/server.rs", "utf8");
    const clientConfig = readFileSync("config/client.toml", "utf8");
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
    expect(
      new URL(requiredMatch(clientConfig, /^endpoint = "([^"]+)"$/mu, "client world endpoint"))
        .pathname,
    ).toBe(WORLD_PATH);
    expect(
      new URL(
        requiredMatch(
          clientConfig,
          /^presence_endpoint = "([^"]+)"$/mu,
          "client presence endpoint",
        ),
      ).pathname,
    ).toBe(PRESENCE_PATH);
    expect(requiredMatch(clientConfig, /^subprotocol = "([^"]+)"$/mu, "client subprotocol")).toBe(
      WORLD_SUBPROTOCOL,
    );
    expect(
      Number(
        requiredMatch(
          streamingDocs,
          /Surface meshes use a separate `VXST` v(\d+) payload/u,
          "documented surface snapshot version",
        ),
      ),
    ).toBe(
      Number(
        requiredMatch(
          protocolSource,
          /const SURFACE_SNAPSHOT_VERSION: u16 = (\d+);/u,
          "Rust surface snapshot version",
        ),
      ),
    );
  });
});
