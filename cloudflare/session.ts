const IDENTITY_CREDENTIAL_VERSION = "vxi1";
const WORLD_SESSION_VERSION = "vxs1";
const IDENTITY_LIFETIME_SECONDS = 366 * 24 * 60 * 60;
const SESSION_LIFETIME_SECONDS = 12 * 60 * 60;
const PLAYER_NAME_PATTERN = /^[a-z0-9][a-z0-9_-]{0,31}$/u;

export function isValidPlayerName(value: string): boolean {
  return PLAYER_NAME_PATTERN.test(value);
}

interface IdentityClaims {
  browserUserId: string;
  playerId: string;
  playerName: string;
}

export interface SessionCredentials extends IdentityClaims {
  authSubprotocolToken: string;
  identityCredential: string;
  expiresAt: number;
}

export async function issueSessionCredentials(
  sessionSigningKey: string,
  identitySigningKey: string,
  playerName: string,
  identityCredential: string | undefined,
  nowSeconds = Math.floor(Date.now() / 1_000),
): Promise<SessionCredentials | null> {
  if (!isValidPlayerName(playerName)) return null;
  if (sessionSigningKey.length < 32 || identitySigningKey.length < 32) {
    throw new Error("session signing keys must be at least 32 characters");
  }
  const [sessionKey, identityKey] = await Promise.all([
    importSigningKey(sessionSigningKey),
    importSigningKey(identitySigningKey),
  ]);
  const identity = identityCredential
    ? await verifyIdentityCredential(identityKey, identityCredential, playerName, nowSeconds)
    : {
        browserUserId: crypto.randomUUID(),
        playerId: crypto.randomUUID(),
        playerName,
      };
  if (identity === null) return null;

  const identityExpiresAt = nowSeconds + IDENTITY_LIFETIME_SECONDS;
  const identityPayload = [
    IDENTITY_CREDENTIAL_VERSION,
    identityExpiresAt.toString(36),
    uuidToBase64Url(identity.browserUserId),
    uuidToBase64Url(identity.playerId),
    textToBase64Url(identity.playerName),
  ].join(".");
  const refreshedIdentityCredential = `${identityPayload}.${await sign(identityKey, identityPayload)}`;

  const expiresAt = nowSeconds + SESSION_LIFETIME_SECONDS;
  const nonce = new Uint8Array(12);
  crypto.getRandomValues(nonce);
  const sessionPayload = [
    WORLD_SESSION_VERSION,
    expiresAt.toString(36),
    uuidToBase64Url(identity.browserUserId),
    uuidToBase64Url(identity.playerId),
    bytesToBase64Url(nonce),
  ].join(".");
  const authSubprotocolToken = `${sessionPayload}.${await sign(sessionKey, sessionPayload)}`;
  if (authSubprotocolToken.length > 128) {
    throw new Error("generated session token exceeds the protocol limit");
  }

  return {
    ...identity,
    authSubprotocolToken,
    identityCredential: refreshedIdentityCredential,
    expiresAt,
  };
}

async function verifyIdentityCredential(
  key: CryptoKey,
  credential: string,
  requestedPlayerName: string,
  nowSeconds: number,
): Promise<IdentityClaims | null> {
  const parts = credential.split(".");
  if (parts.length !== 6 || parts[0] !== IDENTITY_CREDENTIAL_VERSION) return null;
  const [version, expiryText, browserText, playerText, nameText, signature = ""] = parts;
  const expiry = Number.parseInt(expiryText ?? "", 36);
  if (!Number.isSafeInteger(expiry) || expiry <= nowSeconds) return null;
  const payload = [version, expiryText, browserText, playerText, nameText].join(".");
  if (!(await verify(key, payload, signature))) return null;
  try {
    const browserUserId = base64UrlToUuid(browserText ?? "");
    const playerId = base64UrlToUuid(playerText ?? "");
    const playerName = base64UrlToText(nameText ?? "");
    if (playerName !== requestedPlayerName || !isValidPlayerName(playerName)) return null;
    return { browserUserId, playerId, playerName };
  } catch {
    return null;
  }
}

async function importSigningKey(value: string): Promise<CryptoKey> {
  return crypto.subtle.importKey(
    "raw",
    new TextEncoder().encode(value),
    { name: "HMAC", hash: "SHA-256" },
    false,
    ["sign", "verify"],
  );
}

async function sign(key: CryptoKey, payload: string): Promise<string> {
  return bytesToBase64Url(
    new Uint8Array(await crypto.subtle.sign("HMAC", key, new TextEncoder().encode(payload))),
  );
}

async function verify(key: CryptoKey, payload: string, signature: string): Promise<boolean> {
  try {
    const signatureBytes = Uint8Array.from(base64UrlToBytes(signature));
    return crypto.subtle.verify(
      "HMAC",
      key,
      signatureBytes.buffer,
      new TextEncoder().encode(payload),
    );
  } catch {
    return false;
  }
}

function uuidToBase64Url(uuid: string): string {
  const compact = uuid.replaceAll("-", "");
  if (!/^[0-9a-f]{32}$/u.test(compact) || /^0{32}$/u.test(compact)) {
    throw new Error("invalid UUID");
  }
  return bytesToBase64Url(
    Uint8Array.from({ length: 16 }, (_, index) =>
      Number.parseInt(compact.slice(index * 2, index * 2 + 2), 16),
    ),
  );
}

function base64UrlToUuid(value: string): string {
  const bytes = base64UrlToBytes(value);
  if (bytes.length !== 16 || bytes.every((byte) => byte === 0)) throw new Error("invalid UUID");
  const hex = Array.from(bytes, (byte) => byte.toString(16).padStart(2, "0")).join("");
  return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`;
}

function textToBase64Url(value: string): string {
  return bytesToBase64Url(new TextEncoder().encode(value));
}

function base64UrlToText(value: string): string {
  return new TextDecoder("utf-8", { fatal: true }).decode(base64UrlToBytes(value));
}

function bytesToBase64Url(bytes: Uint8Array): string {
  let binary = "";
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return btoa(binary).replaceAll("+", "-").replaceAll("/", "_").replace(/=+$/u, "");
}

function base64UrlToBytes(value: string): Uint8Array {
  if (!/^[A-Za-z0-9_-]+$/u.test(value)) throw new Error("invalid base64url");
  const padded = value
    .replaceAll("-", "+")
    .replaceAll("_", "/")
    .padEnd(Math.ceil(value.length / 4) * 4, "=");
  const binary = atob(padded);
  return Uint8Array.from(binary, (character) => character.charCodeAt(0));
}
