export type FetchConfigText = (url: URL) => Promise<string>;
export type FetchConfigResponse = (url: URL, init: RequestInit) => Promise<Response>;

// Keep the path indirect so Vite does not inline deployment configuration into the JS bundle.
const CLIENT_CONFIG_RELATIVE_PATH = "../config/client.toml";

export function clientConfigUrl(baseUrl: string | URL = import.meta.url): URL {
  return new URL(CLIENT_CONFIG_RELATIVE_PATH, baseUrl);
}

export async function fetchConfigText(
  url: URL,
  fetchResponse: FetchConfigResponse = fetch,
): Promise<string> {
  // This is deployment configuration at a stable URL. Always re-read it on page startup so a
  // reload observes an operator edit instead of an older browser/CDN cache entry.
  const response = await fetchResponse(url, { cache: "no-store" });
  if (!response.ok) {
    throw new Error(`client config request failed (${response.status} ${response.statusText})`);
  }
  return response.text();
}

/** Loads the client-owned TOML without duplicating its Rust schema in TypeScript. */
export async function loadClientConfig(
  fetchText: FetchConfigText = fetchConfigText,
  url: URL = clientConfigUrl(),
): Promise<string> {
  const contents = await fetchText(url);
  if (contents.trim().length === 0) throw new Error("client config is empty");
  return contents;
}
