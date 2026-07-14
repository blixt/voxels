import { describe, expect, it } from "vite-plus/test";
import { clientConfigUrl, fetchConfigText, loadClientConfig } from "./client-config.ts";

describe("client config loading", () => {
  it("injects config I/O for tests instead of replacing global fetch", async () => {
    const seen: URL[] = [];
    const configUrl = new URL("https://example.test/game/config/client.toml");
    const contents = await loadClientConfig(async (url) => {
      seen.push(url);
      return "schema_version = 1\n";
    }, configUrl);

    expect(contents).toBe("schema_version = 1\n");
    expect(seen).toEqual([configUrl]);
  });

  it("resolves beside the deployed asset tree without bundling the file", () => {
    expect(clientConfigUrl("https://example.test/game/assets/index.js").href).toBe(
      "https://example.test/game/config/client.toml",
    );
  });

  it("revalidates the stable deployment file on every startup", async () => {
    const configUrl = new URL("https://example.test/game/config/client.toml");
    let seenCache: RequestCache | undefined;
    const contents = await fetchConfigText(configUrl, async (_url, init) => {
      seenCache = init.cache;
      return new Response("schema_version = 1\n");
    });

    expect(contents).toBe("schema_version = 1\n");
    expect(seenCache).toBe("no-store");
  });

  it("rejects an empty file before engine startup", async () => {
    await expect(loadClientConfig(async () => "  \n")).rejects.toThrow("client config is empty");
  });
});
