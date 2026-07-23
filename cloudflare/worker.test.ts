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

function rawSessionRequest(
  body: BodyInit | null,
  headers: Record<string, string> = {},
): Parameters<typeof worker.fetch>[0] {
  return new Request("https://voxels.lol/api/session", {
    method: "POST",
    headers: {
      "CF-Connecting-IP": "203.0.113.10",
      "Content-Type": "application/json",
      Origin: "https://voxels.lol",
      "Sec-Fetch-Site": "same-origin",
      ...headers,
    },
    body,
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

  it("classifies malformed and oversized bodies as request errors", async () => {
    const env = testEnv(true, []);
    const invalidUtf8 = new Uint8Array([
      ...new TextEncoder().encode('{"playerName":"default","extra":"'),
      0xff,
      ...new TextEncoder().encode('"}'),
    ]);
    for (const body of ["", " \n\t", '{"playerName":', invalidUtf8]) {
      const malformed = await worker.fetch(rawSessionRequest(body), env);
      expect(malformed.status).toBe(400);
      await expect(malformed.json()).resolves.toEqual({ error: "Invalid session request" });
    }

    const streamedOversize = await worker.fetch(
      rawSessionRequest(JSON.stringify({ playerName: "a".repeat(4_096) })),
      env,
    );
    expect(streamedOversize.status).toBe(413);
    await expect(streamedOversize.json()).resolves.toEqual({
      error: "Session request is too large",
    });

    const declaredOversize = await worker.fetch(
      rawSessionRequest("{}", { "Content-Length": "4097" }),
      env,
    );
    expect(declaredOversize.status).toBe(413);

    const exactBody = '{"playerName":"default"}'.padEnd(4_096, " ");
    const exactLimit = await worker.fetch(rawSessionRequest(exactBody), env);
    expect(exactLimit.status).toBe(200);
  });

  it("requires JSON and rejects invalid request fields before rate limiting", async () => {
    const keys: string[] = [];
    const env = testEnv(true, keys);
    const missingType = await worker.fetch(rawSessionRequest("{}", { "Content-Type": "" }), env);
    expect(missingType.status).toBe(415);
    const text = await worker.fetch(rawSessionRequest("{}", { "Content-Type": "text/plain" }), env);
    expect(text.status).toBe(415);
    const jsonWithCharset = await worker.fetch(
      rawSessionRequest('{"playerName":"default"}', {
        "Content-Type": "Application/JSON; Charset=UTF-8",
      }),
      env,
    );
    expect(jsonWithCharset.status).toBe(200);

    for (const playerName of ["", "Alice", "a".repeat(33)]) {
      const invalidName = await worker.fetch(sessionRequest({ playerName }), env);
      expect(invalidName.status).toBe(400);
    }
    for (const identityCredential of [42, "a".repeat(513)]) {
      const invalidCredential = await worker.fetch(
        sessionRequest({ identityCredential, playerName: "default" }),
        env,
      );
      expect(invalidCredential.status).toBe(400);
    }
    expect(keys).toEqual(["new-identity:203.0.113.10"]);
  });
});
