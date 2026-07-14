import { describe, expect, it } from "vite-plus/test";
import {
  LOCAL_PLAYER_REGISTRY_KEY,
  namedPlayerUrl,
  playerNameFromUrl,
  resolveBrowserPlayerSession,
  type IdentityLock,
  type LocalPlayerStorage,
} from "./local-player.ts";

const IDS = [
  "00000000-0000-4000-8000-000000000001",
  "00000000-0000-4000-8000-000000000002",
  "00000000-0000-4000-8000-000000000003",
  "00000000-0000-4000-8000-000000000004",
];

class MemoryStorage implements LocalPlayerStorage {
  readonly values = new Map<string, string>();

  getItem(key: string): string | null {
    return this.values.get(key) ?? null;
  }

  setItem(key: string, value: string): void {
    this.values.set(key, value);
  }
}

function uuidSequence(): () => string {
  let index = 0;
  return () => IDS[index++] ?? "00000000-0000-4000-8000-000000000099";
}

const immediateLock: IdentityLock = async (operation) => operation();

describe("local browser players", () => {
  it("creates and reuses the automatic default player", async () => {
    const storage = new MemoryStorage();
    const randomUuid = uuidSequence();
    const url = new URL("http://127.0.0.1:5173/");
    const first = await resolveBrowserPlayerSession(url, storage, randomUuid, immediateLock);
    const second = await resolveBrowserPlayerSession(url, storage, randomUuid, immediateLock);

    expect(first).toEqual({
      browserUserId: IDS[0],
      playerId: IDS[1],
      playerName: "default",
    });
    expect(second).toEqual(first);
  });

  it("keeps named players stable and distinct under one browser user", async () => {
    const storage = new MemoryStorage();
    const randomUuid = uuidSequence();
    const aliceUrl = new URL("http://127.0.0.1:5173/?player=alice");
    const bobUrl = new URL("http://127.0.0.1:5173/?player=bob");
    const alice = await resolveBrowserPlayerSession(aliceUrl, storage, randomUuid, immediateLock);
    const bob = await resolveBrowserPlayerSession(bobUrl, storage, randomUuid, immediateLock);
    const restoredAlice = await resolveBrowserPlayerSession(
      aliceUrl,
      storage,
      randomUuid,
      immediateLock,
    );

    expect(alice.browserUserId).toBe(bob.browserUserId);
    expect(alice.playerId).not.toBe(bob.playerId);
    expect(restoredAlice).toEqual(alice);
  });

  it("rejects ambiguous names and keeps ids out of generated urls", () => {
    expect(() => playerNameFromUrl(new URL("http://local/?player=Alice"))).toThrow("lowercase");
    expect(() => playerNameFromUrl(new URL("http://local/?player=a&player=b"))).toThrow(
      "exactly one",
    );
    expect(namedPlayerUrl("alice", "http://local/game?debug=1").href).toBe(
      "http://local/game?debug=1&player=alice",
    );
    expect(namedPlayerUrl("default", "http://local/?player=alice").href).toBe("http://local/");
  });

  it("fails closed without overwriting corrupt or unavailable durable storage", async () => {
    const corrupt = new MemoryStorage();
    corrupt.values.set(LOCAL_PLAYER_REGISTRY_KEY, "not json");
    await expect(
      resolveBrowserPlayerSession(new URL("http://local/"), corrupt, uuidSequence(), immediateLock),
    ).rejects.toThrow("not valid JSON");
    expect(corrupt.values.get(LOCAL_PLAYER_REGISTRY_KEY)).toBe("not json");

    const unavailable: LocalPlayerStorage = {
      getItem: () => null,
      setItem: () => {
        throw new Error("quota denied");
      },
    };
    await expect(
      resolveBrowserPlayerSession(
        new URL("http://local/"),
        unavailable,
        uuidSequence(),
        immediateLock,
      ),
    ).rejects.toThrow("Could not persist");
  });

  it("serializes simultaneous first opens through the injected identity lock", async () => {
    const storage = new MemoryStorage();
    const randomUuid = uuidSequence();
    let tail = Promise.resolve();
    const lock: IdentityLock = async (operation) => {
      let release = (): void => undefined;
      const previous = tail;
      tail = new Promise<void>((resolve) => {
        release = resolve;
      });
      await previous;
      try {
        return await operation();
      } finally {
        release();
      }
    };
    const url = new URL("http://local/?player=alice");
    const [first, second] = await Promise.all([
      resolveBrowserPlayerSession(url, storage, randomUuid, lock),
      resolveBrowserPlayerSession(url, storage, randomUuid, lock),
    ]);
    expect(second).toEqual(first);
  });
});
