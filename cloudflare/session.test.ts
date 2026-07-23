import { describe, expect, it } from "vite-plus/test";
import { issueSessionCredentials } from "./session.ts";

const SESSION_SIGNING_KEY = "test-only-session-signing-key-that-is-long-enough";
const IDENTITY_SIGNING_KEY = "test-only-identity-signing-key-that-is-long-enough";
const ROTATED_SESSION_SIGNING_KEY = "rotated-session-signing-key-that-is-also-long-enough";
const NOW = 1_800_000_000;

describe("public session credentials", () => {
  it("mints a bounded WebSocket token and refreshes the same durable identity", async () => {
    const first = await issueSessionCredentials(
      SESSION_SIGNING_KEY,
      IDENTITY_SIGNING_KEY,
      "default",
      undefined,
      NOW,
    );
    expect(first).not.toBeNull();
    expect(first?.authSubprotocolToken).toMatch(/^vxs1\./u);
    expect(first?.authSubprotocolToken.length).toBeLessThanOrEqual(128);

    const refreshed = await issueSessionCredentials(
      ROTATED_SESSION_SIGNING_KEY,
      IDENTITY_SIGNING_KEY,
      "default",
      first?.identityCredential,
      NOW + 60,
    );
    expect(refreshed?.browserUserId).toBe(first?.browserUserId);
    expect(refreshed?.playerId).toBe(first?.playerId);
    expect(refreshed?.identityCredential).not.toBe(first?.identityCredential);
  });

  it("rejects forged, expired, renamed, and malformed identity credentials", async () => {
    const first = await issueSessionCredentials(
      SESSION_SIGNING_KEY,
      IDENTITY_SIGNING_KEY,
      "alice",
      undefined,
      NOW,
    );
    expect(first).not.toBeNull();
    const credential = first?.identityCredential ?? "";
    const credentialParts = credential.split(".");
    const signature = credentialParts.at(-1) ?? "";
    credentialParts[credentialParts.length - 1] =
      `${signature.startsWith("A") ? "B" : "A"}${signature.slice(1)}`;
    const forged = credentialParts.join(".");

    await expect(
      issueSessionCredentials(SESSION_SIGNING_KEY, IDENTITY_SIGNING_KEY, "alice", forged, NOW + 1),
    ).resolves.toBeNull();
    await expect(
      issueSessionCredentials(
        SESSION_SIGNING_KEY,
        IDENTITY_SIGNING_KEY,
        "alice",
        credential,
        NOW + 367 * 24 * 60 * 60,
      ),
    ).resolves.toBeNull();
    await expect(
      issueSessionCredentials(
        SESSION_SIGNING_KEY,
        IDENTITY_SIGNING_KEY,
        "bob",
        credential,
        NOW + 1,
      ),
    ).resolves.toBeNull();
    await expect(
      issueSessionCredentials(SESSION_SIGNING_KEY, IDENTITY_SIGNING_KEY, "Alice", undefined, NOW),
    ).resolves.toBeNull();
  });

  it("treats invalid signing-key configuration as an operational failure", async () => {
    await expect(
      issueSessionCredentials("too-short", IDENTITY_SIGNING_KEY, "default", undefined, NOW),
    ).rejects.toThrow("session signing keys must be at least 32 characters");
  });
});
