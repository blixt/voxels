import { describe, expect, it } from "vite-plus/test";
import worker from "./worker.ts";

const SESSION_SIGNING_KEY = "test-only-session-signing-key-that-is-long-enough";
const IDENTITY_SIGNING_KEY = "test-only-identity-signing-key-that-is-long-enough";

function sessionRequest(body: unknown): Parameters<typeof worker.fetch>[0] {
  return new Request("https://voxels.lol/api/session", {
    method: "POST",
    headers: {
      "CF-Connecting-IP": "203.0.113.10",
      "Content-Type": "application/json",
      Origin: "https://voxels.lol",
      "Sec-Fetch-Site": "same-origin",
    },
    body: JSON.stringify(body),
  }) as Parameters<typeof worker.fetch>[0];
}

function testEnv(success: boolean, keys: string[]): Env {
  return {
    IDENTITY_ISSUANCE_RATE_LIMITER: {
      limit: async ({ key }: { key: string }) => {
        keys.push(key);
        return { success };
      },
    },
    VOXELS_IDENTITY_SIGNING_KEY: IDENTITY_SIGNING_KEY,
    VOXELS_SESSION_SIGNING_KEY: SESSION_SIGNING_KEY,
  } as unknown as Env;
}

describe("session Worker", () => {
  it("rate-limits only requests that mint a new durable identity", async () => {
    const keys: string[] = [];
    const env = testEnv(true, keys);
    const first = await worker.fetch(sessionRequest({ playerName: "default" }), env);
    expect(first.status).toBe(200);
    const identityCredential = ((await first.json()) as { identityCredential: string })
      .identityCredential;
    const refreshed = await worker.fetch(
      sessionRequest({ identityCredential, playerName: "default" }),
      env,
    );

    expect(refreshed.status).toBe(200);
    expect(keys).toEqual(["new-identity:203.0.113.10"]);
  });

  it("returns retry guidance when new identity issuance is exhausted", async () => {
    const response = await worker.fetch(
      sessionRequest({ playerName: "default" }),
      testEnv(false, []),
    );

    expect(response.status).toBe(429);
    expect(response.headers.get("Retry-After")).toBe("60");
  });
});
