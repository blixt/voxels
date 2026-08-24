import { describe, expect, it } from "vite-plus/test";
import { authorizeClientBootstrap, type PublicIdentityLock } from "./client-authorization.ts";
import type { BrowserPlayerSession, LocalPlayerStorage } from "./local-player.ts";

const LOCAL_PLAYER: BrowserPlayerSession = {
  browserUserId: "00000000-0000-4000-8000-000000000001",
  playerId: "00000000-0000-4000-8000-000000000002",
  playerName: "default",
};
const NOW_SECONDS = 1_800_000_000;
const VALID_EXPIRY = NOW_SECONDS + 12 * 60 * 60;

class MemoryStorage implements LocalPlayerStorage {
  readonly values = new Map<string, string>();

  getItem(key: string): string | null {
    return this.values.get(key) ?? null;
  }

  setItem(key: string, value: string): void {
    this.values.set(key, value);
  }
}

function serialIdentityLock(): PublicIdentityLock {
  const tails = new Map<string, Promise<void>>();
  return async (storageKey, operation) => {
    const previous = tails.get(storageKey) ?? Promise.resolve();
    let release = (): void => undefined;
    const current = new Promise<void>((resolve) => {
      release = resolve;
    });
    tails.set(storageKey, current);
    await previous;
    try {
      return await operation();
    } finally {
      release();
      if (tails.get(storageKey) === current) tails.delete(storageKey);
    }
  };
}

describe("public client authorization", () => {
  it("leaves local static-token configuration and identity untouched", async () => {
    const configToml = 'auth_subprotocol_token = "local-token"\n';
    let fetched = false;
    const result = await authorizeClientBootstrap(
      configToml,
      LOCAL_PLAYER,
      "http://127.0.0.1:5173/",
      new MemoryStorage(),
      async () => {
        fetched = true;
        return new Response();
      },
    );
    expect(fetched).toBe(false);
    expect(result).toEqual({ configToml, player: LOCAL_PLAYER });
  });

  it("rejects a cross-origin session marker without sending stored identity", async () => {
    const storage = new MemoryStorage();
    storage.setItem("voxels.public-identity.v1.default", "vxi1.private-credential");
    let fetched = false;
    await expect(
      authorizeClientBootstrap(
        'auth_subprotocol_token = "session:https://attacker.test/api/session"\n',
        LOCAL_PLAYER,
        "https://voxels.lol/play",
        storage,
        async () => {
          fetched = true;
          return new Response();
        },
      ),
    ).rejects.toThrow("session authorization endpoint must be same-origin");
    expect(fetched).toBe(false);
  });

  it("exchanges the deployment marker for a signed token and server-owned identity", async () => {
    const storage = new MemoryStorage();
    const configToml = 'auth_subprotocol_token = "session:/api/session" # deployment\n';
    const serverPlayer = {
      browserUserId: "00000000-0000-4000-8000-000000000010",
      playerId: "00000000-0000-4000-8000-000000000011",
      playerName: "default",
    };
    let requestBody: unknown;
    const result = await authorizeClientBootstrap(
      configToml,
      LOCAL_PLAYER,
      "https://voxels.lol/play",
      storage,
      async (url, init) => {
        expect(url).toBeInstanceOf(URL);
        expect((url as URL).href).toBe("https://voxels.lol/api/session");
        expect(init?.method).toBe("POST");
        expect(typeof init?.body).toBe("string");
        requestBody = JSON.parse(init?.body as string) as unknown;
        return Response.json({
          ...serverPlayer,
          authSubprotocolToken: "vxs1.signed-token",
          identityCredential: "vxi1.durable-credential",
          expiresAt: VALID_EXPIRY,
        });
      },
      NOW_SECONDS,
    );

    expect(requestBody).toEqual({ playerName: "default" });
    expect(result.player).toEqual(serverPlayer);
    expect(result.configToml).toContain('auth_subprotocol_token = "vxs1.signed-token"');
    expect(result.sessionExpiresAt).toBe(VALID_EXPIRY);
    expect(storage.values.get("voxels.public-identity.v1.default")).toBe("vxi1.durable-credential");
  });

  it("inserts every valid WebSocket token character literally", async () => {
    const result = await authorizeClientBootstrap(
      'auth_subprotocol_token = "session:/api/session" # deployment\n',
      LOCAL_PLAYER,
      "https://voxels.lol/",
      new MemoryStorage(),
      async () =>
        Response.json({
          ...LOCAL_PLAYER,
          authSubprotocolToken: "vxs1.$&-signed",
          identityCredential: "vxi1.durable-credential",
          expiresAt: VALID_EXPIRY,
        }),
      NOW_SECONDS,
    );

    expect(result.configToml).toBe('auth_subprotocol_token = "vxs1.$&-signed" # deployment\n');
  });

  it("reissues an identity once when a stored credential is no longer valid", async () => {
    const storage = new MemoryStorage();
    storage.setItem("voxels.public-identity.v1.default", "vxi1.expired-credential");
    const requestBodies: unknown[] = [];
    const result = await authorizeClientBootstrap(
      'auth_subprotocol_token = "session:/api/session"\n',
      LOCAL_PLAYER,
      "https://voxels.lol/",
      storage,
      async (_url, init) => {
        requestBodies.push(JSON.parse(init?.body as string) as unknown);
        if (requestBodies.length === 1) return new Response(null, { status: 401 });
        return Response.json({
          browserUserId: "00000000-0000-4000-8000-000000000020",
          playerId: "00000000-0000-4000-8000-000000000021",
          playerName: "default",
          authSubprotocolToken: "vxs1.reissued-token",
          identityCredential: "vxi1.reissued-credential",
          expiresAt: VALID_EXPIRY,
        });
      },
      NOW_SECONDS,
    );

    expect(requestBodies).toEqual([
      { identityCredential: "vxi1.expired-credential", playerName: "default" },
      { playerName: "default" },
    ]);
    expect(result.player.playerId).toBe("00000000-0000-4000-8000-000000000021");
    expect(storage.values.get("voxels.public-identity.v1.default")).toBe(
      "vxi1.reissued-credential",
    );
  });

  it("serializes simultaneous first loads before issuing a durable identity", async () => {
    const storage = new MemoryStorage();
    const requestBodies: Array<Record<string, unknown>> = [];
    let releaseFirstResponse = (): void => undefined;
    const firstResponseGate = new Promise<void>((resolve) => {
      releaseFirstResponse = resolve;
    });
    const fetchResponse: typeof fetch = async (_url, init) => {
      const body = JSON.parse(init?.body as string) as Record<string, unknown>;
      requestBodies.push(body);
      if (requestBodies.length === 1) await firstResponseGate;
      return Response.json({
        browserUserId: "00000000-0000-4000-8000-000000000030",
        playerId: "00000000-0000-4000-8000-000000000031",
        playerName: "default",
        authSubprotocolToken: `vxs1.signed-token-${requestBodies.length}`,
        identityCredential: "vxi1.shared-durable-credential",
        expiresAt: VALID_EXPIRY,
      });
    };
    const lock = serialIdentityLock();
    const configToml = 'auth_subprotocol_token = "session:/api/session"\n';

    const first = authorizeClientBootstrap(
      configToml,
      LOCAL_PLAYER,
      "https://voxels.lol/",
      storage,
      fetchResponse,
      NOW_SECONDS,
      lock,
    );
    await Promise.resolve();
    const second = authorizeClientBootstrap(
      configToml,
      LOCAL_PLAYER,
      "https://voxels.lol/",
      storage,
      fetchResponse,
      NOW_SECONDS,
      lock,
    );
    await Promise.resolve();
    expect(requestBodies).toEqual([{ playerName: "default" }]);

    releaseFirstResponse();
    const [firstResult, secondResult] = await Promise.all([first, second]);

    expect(secondResult.player).toEqual(firstResult.player);
    expect(requestBodies).toEqual([
      { playerName: "default" },
      { identityCredential: "vxi1.shared-durable-credential", playerName: "default" },
    ]);
  });

  it("rejects mismatched and nil session identities", async () => {
    for (const identity of [
      {
        browserUserId: "00000000-0000-4000-8000-000000000010",
        playerId: "00000000-0000-4000-8000-000000000011",
        playerName: "alice",
      },
      {
        browserUserId: "00000000-0000-0000-0000-000000000000",
        playerId: "00000000-0000-4000-8000-000000000011",
        playerName: "default",
      },
      {
        browserUserId: "00000000-0000-4000-8000-000000000010",
        playerId: "00000000-0000-0000-0000-000000000000",
        playerName: "default",
      },
    ]) {
      const storage = new MemoryStorage();
      await expect(
        authorizeClientBootstrap(
          'auth_subprotocol_token = "session:/api/session"\n',
          LOCAL_PLAYER,
          "https://voxels.lol/",
          storage,
          async () =>
            Response.json({
              ...identity,
              authSubprotocolToken: "vxs1.signed-token",
              identityCredential: "vxi1.durable-credential",
              expiresAt: VALID_EXPIRY,
            }),
          NOW_SECONDS,
        ),
      ).rejects.toThrow("session authorization returned invalid credentials");
      expect(storage.values.size).toBe(0);
    }
  });

  it("rejects expiries that would reload immediately or outlive the server window", async () => {
    for (const expiresAt of [
      NOW_SECONDS - 1,
      NOW_SECONDS + 5 * 60,
      NOW_SECONDS + 13 * 60 * 60 + 1,
    ]) {
      await expect(
        authorizeClientBootstrap(
          'auth_subprotocol_token = "session:/api/session"\n',
          LOCAL_PLAYER,
          "https://voxels.lol/",
          new MemoryStorage(),
          async () =>
            Response.json({
              ...LOCAL_PLAYER,
              authSubprotocolToken: "vxs1.signed-token",
              identityCredential: "vxi1.durable-credential",
              expiresAt,
            }),
          NOW_SECONDS,
        ),
      ).rejects.toThrow("session authorization returned invalid credentials");
    }
  });
});
