import type { BrowserPlayerSession, LocalPlayerStorage } from "./local-player.ts";

const SESSION_TOKEN_PREFIX = "session:";
const AUTH_TOKEN_LINE = /^(auth_subprotocol_token\s*=\s*")([^"]+)("\s*(?:#.*)?)$/mu;
const WEBSOCKET_TOKEN = /^[!#$%&'*+\-.^_`|~0-9A-Za-z]+$/u;
const UUID_PATTERN = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/u;

interface SessionResponse extends BrowserPlayerSession {
  authSubprotocolToken: string;
  identityCredential: string;
  expiresAt: number;
}

export interface AuthorizedClientBootstrap {
  configToml: string;
  player: BrowserPlayerSession;
  sessionExpiresAt?: number;
}

export async function authorizeClientBootstrap(
  configToml: string,
  localPlayer: BrowserPlayerSession,
  baseUrl: string | URL = location.href,
  storage: LocalPlayerStorage = localStorage,
  fetchResponse: typeof fetch = fetch,
): Promise<AuthorizedClientBootstrap> {
  const match = AUTH_TOKEN_LINE.exec(configToml);
  const configuredToken = match?.[2];
  if (configuredToken === undefined || !configuredToken.startsWith(SESSION_TOKEN_PREFIX)) {
    return { configToml, player: localPlayer };
  }
  const endpoint = new URL(configuredToken.slice(SESSION_TOKEN_PREFIX.length), baseUrl);
  const storageKey = `voxels.public-identity.v1.${localPlayer.playerName}`;
  const identityCredential = storage.getItem(storageKey) ?? undefined;
  let response = await requestSession(
    endpoint,
    localPlayer.playerName,
    identityCredential,
    fetchResponse,
  );
  if (response.status === 401 && identityCredential !== undefined) {
    response = await requestSession(endpoint, localPlayer.playerName, undefined, fetchResponse);
  }
  if (!response.ok) {
    throw new Error(`session authorization failed (${response.status} ${response.statusText})`);
  }
  const value = validateSessionResponse(await response.json());
  try {
    storage.setItem(storageKey, value.identityCredential);
  } catch (error) {
    throw new Error(`Could not persist the public player credential: ${String(error)}`);
  }
  return {
    configToml: configToml.replace(AUTH_TOKEN_LINE, `$1${value.authSubprotocolToken}$3`),
    player: {
      browserUserId: value.browserUserId,
      playerId: value.playerId,
      playerName: value.playerName,
    },
    sessionExpiresAt: value.expiresAt,
  };
}

async function requestSession(
  endpoint: URL,
  playerName: string,
  identityCredential: string | undefined,
  fetchResponse: typeof fetch,
): Promise<Response> {
  return fetchResponse(endpoint, {
    method: "POST",
    cache: "no-store",
    credentials: "same-origin",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ identityCredential, playerName }),
  });
}

function validateSessionResponse(value: unknown): SessionResponse {
  if (!isRecord(value)) throw new Error("session authorization returned invalid JSON");
  const authSubprotocolToken = requiredString(value.authSubprotocolToken, "session token");
  const identityCredential = requiredString(value.identityCredential, "identity credential");
  const browserUserId = requiredString(value.browserUserId, "browser user id");
  const playerId = requiredString(value.playerId, "player id");
  const playerName = requiredString(value.playerName, "player name");
  if (
    !WEBSOCKET_TOKEN.test(authSubprotocolToken) ||
    authSubprotocolToken.length > 128 ||
    identityCredential.length > 512 ||
    !UUID_PATTERN.test(browserUserId) ||
    !UUID_PATTERN.test(playerId) ||
    typeof value.expiresAt !== "number" ||
    !Number.isSafeInteger(value.expiresAt)
  ) {
    throw new Error("session authorization returned invalid credentials");
  }
  return {
    authSubprotocolToken,
    identityCredential,
    browserUserId,
    playerId,
    playerName,
    expiresAt: value.expiresAt,
  };
}

function requiredString(value: unknown, field: string): string {
  if (typeof value !== "string" || value.length === 0) {
    throw new Error(`session authorization omitted ${field}`);
  }
  return value;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
