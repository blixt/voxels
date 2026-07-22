export const LOCAL_PLAYER_REGISTRY_VERSION = 1;
export const LOCAL_PLAYER_REGISTRY_KEY = "voxels.local-players.v1";
export const DEFAULT_PLAYER_NAME = "default";

const PLAYER_NAME_PATTERN = /^[a-z0-9][a-z0-9_-]{0,31}$/;
const UUID_PATTERN = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;

export interface BrowserPlayerSession {
  browserUserId: string;
  playerId: string;
  playerName: string;
}

export interface LocalPlayerStorage {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
}

interface StoredPlayerRegistry {
  schemaVersion: number;
  browserUserId: string;
  players: Record<string, string>;
}

export type IdentityLock = <T>(operation: () => Promise<T> | T) => Promise<T>;

export function playerNameFromUrl(url: URL): string {
  const names = url.searchParams.getAll("player");
  if (names.length === 0) return DEFAULT_PLAYER_NAME;
  if (names.length !== 1) throw new Error("Specify exactly one ?player=name parameter.");
  return validatePlayerName(names[0] ?? "");
}

export function namedPlayerUrl(name: string, baseUrl: string | URL = location.href): URL {
  const url = new URL(baseUrl);
  const validated = validatePlayerName(name);
  if (validated === DEFAULT_PLAYER_NAME) {
    url.searchParams.delete("player");
  } else {
    url.searchParams.set("player", validated);
  }
  return url;
}

export async function resolveBrowserPlayerSession(
  url: URL = new URL(location.href),
  storage: LocalPlayerStorage = localStorage,
  randomUuid: () => string = () => crypto.randomUUID(),
  lock: IdentityLock = withBrowserIdentityLock,
): Promise<BrowserPlayerSession> {
  const playerName = playerNameFromUrl(url);
  return lock(async () => {
    const stored = storage.getItem(LOCAL_PLAYER_REGISTRY_KEY);
    const registry = stored === null ? createRegistry(randomUuid) : parseRegistry(stored);
    let changed = stored === null;
    let playerId = Object.hasOwn(registry.players, playerName)
      ? registry.players[playerName]
      : undefined;
    if (playerId === undefined) {
      playerId = checkedUuid(randomUuid(), "generated player id");
      if (Object.values(registry.players).includes(playerId)) {
        throw new Error("Generated player id collides with an existing local player.");
      }
      registry.players[playerName] = playerId;
      changed = true;
    }
    if (changed) {
      try {
        storage.setItem(LOCAL_PLAYER_REGISTRY_KEY, JSON.stringify(registry));
      } catch (error) {
        throw new Error(`Could not persist the local player registry: ${String(error)}`);
      }
    }
    if (!Object.hasOwn(registry.players, DEFAULT_PLAYER_NAME))
      throw new Error("Local player registry has no default player.");
    return {
      browserUserId: registry.browserUserId,
      playerId,
      playerName,
    };
  });
}

function createRegistry(randomUuid: () => string): StoredPlayerRegistry {
  const browserUserId = checkedUuid(randomUuid(), "generated browser user id");
  const defaultPlayerId = checkedUuid(randomUuid(), "generated default player id");
  if (browserUserId === defaultPlayerId) {
    throw new Error("Generated browser user and default player ids collide.");
  }
  return {
    schemaVersion: LOCAL_PLAYER_REGISTRY_VERSION,
    browserUserId,
    players: { [DEFAULT_PLAYER_NAME]: defaultPlayerId },
  };
}

function parseRegistry(contents: string): StoredPlayerRegistry {
  let value: unknown;
  try {
    value = JSON.parse(contents);
  } catch (error) {
    throw new Error(`Local player registry is not valid JSON: ${String(error)}`);
  }
  if (!isRecord(value) || value.schemaVersion !== LOCAL_PLAYER_REGISTRY_VERSION) {
    throw new Error("Local player registry has an unsupported schema version.");
  }
  const browserUserId = checkedUuid(value.browserUserId, "stored browser user id");
  if (!isRecord(value.players)) throw new Error("Local player registry has no player map.");
  const players: Record<string, string> = {};
  const seen = new Set<string>();
  for (const [name, id] of Object.entries(value.players)) {
    const validatedName = validatePlayerName(name);
    const validatedId = checkedUuid(id, `stored player id for ${validatedName}`);
    if (seen.has(validatedId))
      throw new Error("Local player registry contains duplicate player ids.");
    seen.add(validatedId);
    players[validatedName] = validatedId;
  }
  if (!Object.hasOwn(players, DEFAULT_PLAYER_NAME)) {
    throw new Error("Local player registry has no default player.");
  }
  return {
    schemaVersion: LOCAL_PLAYER_REGISTRY_VERSION,
    browserUserId,
    players,
  };
}

function validatePlayerName(name: string): string {
  if (!PLAYER_NAME_PATTERN.test(name)) {
    throw new Error("Player names must be 1-32 lowercase letters, digits, '_' or '-'.");
  }
  return name;
}

function checkedUuid(value: unknown, field: string): string {
  if (
    typeof value !== "string" ||
    !UUID_PATTERN.test(value) ||
    /^0{8}-0{4}-0{4}-0{4}-0{12}$/.test(value)
  ) {
    throw new Error(`${field} is not a non-nil lowercase UUID.`);
  }
  return value;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

async function withBrowserIdentityLock<T>(operation: () => Promise<T> | T): Promise<T> {
  if (navigator.locks === undefined) return operation();
  return navigator.locks.request(LOCAL_PLAYER_REGISTRY_KEY, { mode: "exclusive" }, operation);
}
