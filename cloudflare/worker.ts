import { isValidPlayerName, issueSessionCredentials } from "./session.ts";

const PUBLIC_ORIGIN = "https://voxels.lol";
const MAX_SESSION_REQUEST_BYTES = 4_096;
const RATE_LIMIT_WINDOW_SECONDS = 60;
const API_CONTENT_SECURITY_POLICY = "default-src 'none'";
const ASSET_CONTENT_SECURITY_POLICY =
  "default-src 'self'; base-uri 'none'; connect-src 'self' wss://voxels-world-blixt.fly.dev; font-src 'self'; form-action 'none'; frame-ancestors 'none'; img-src 'self' data:; object-src 'none'; script-src 'self' 'wasm-unsafe-eval'; style-src 'self'; worker-src 'self'";

class SessionRequestError extends Error {
  readonly status: 400 | 413;

  constructor(status: 400 | 413, message: string) {
    super(message);
    this.name = "SessionRequestError";
    this.status = status;
  }
}

export default {
  async fetch(request, env): Promise<Response> {
    const url = new URL(request.url);
    let response: Response;
    let contentSecurityPolicy = API_CONTENT_SECURITY_POLICY;
    try {
      if (url.pathname === "/api/session") {
        response = await createSession(request, env);
      } else if (url.pathname.startsWith("/api/")) {
        response = Response.json({ error: "Not found" }, { status: 404 });
      } else {
        response = await env.ASSETS.fetch(request);
        contentSecurityPolicy = ASSET_CONTENT_SECURITY_POLICY;
      }
    } catch (error) {
      if (error instanceof SessionRequestError) {
        response = Response.json({ error: error.message }, { status: error.status });
      } else {
        console.error(
          JSON.stringify({
            message: "request failed",
            path: url.pathname,
            error: error instanceof Error ? error.message : String(error),
          }),
        );
        response = Response.json({ error: "Internal server error" }, { status: 500 });
      }
    }
    return withSecurityHeaders(response, contentSecurityPolicy);
  },
} satisfies ExportedHandler<Env>;

async function createSession(request: Request, env: Env): Promise<Response> {
  if (request.method !== "POST") {
    return Response.json(
      { error: "Method not allowed" },
      { status: 405, headers: { Allow: "POST" } },
    );
  }
  if (
    request.headers.get("Origin") !== PUBLIC_ORIGIN ||
    request.headers.get("Sec-Fetch-Site") !== "same-origin"
  ) {
    return Response.json({ error: "Origin not allowed" }, { status: 403 });
  }
  if (!isJsonContentType(request.headers.get("Content-Type"))) {
    return Response.json({ error: "Content-Type must be application/json" }, { status: 415 });
  }
  const body = await readBoundedJson(request);
  if (
    !isRecord(body) ||
    typeof body.playerName !== "string" ||
    !isValidPlayerName(body.playerName)
  ) {
    return Response.json({ error: "Invalid session request" }, { status: 400 });
  }
  const identityCredential =
    body.identityCredential === undefined
      ? undefined
      : typeof body.identityCredential === "string"
        ? body.identityCredential
        : null;
  if (
    identityCredential === null ||
    (identityCredential !== undefined &&
      (identityCredential.length === 0 || identityCredential.length > 512))
  ) {
    return Response.json({ error: "Invalid session request" }, { status: 400 });
  }
  if (identityCredential === undefined) {
    const clientAddress = request.headers.get("CF-Connecting-IP") ?? "missing";
    const { success } = await env.IDENTITY_ISSUANCE_RATE_LIMITER.limit({
      key: `new-identity:${clientAddress}`,
    });
    if (!success) {
      return Response.json(
        { error: "Too many new player identities; try again shortly" },
        {
          status: 429,
          headers: {
            "Cache-Control": "no-store",
            "Retry-After": RATE_LIMIT_WINDOW_SECONDS.toString(),
          },
        },
      );
    }
  }
  const credentials = await issueSessionCredentials(
    env.VOXELS_SESSION_SIGNING_KEY,
    env.VOXELS_IDENTITY_SIGNING_KEY,
    body.playerName,
    identityCredential,
  );
  if (credentials === null) {
    return Response.json({ error: "Identity credential is invalid or expired" }, { status: 401 });
  }
  return Response.json(credentials, {
    headers: {
      "Cache-Control": "no-store",
    },
  });
}

async function readBoundedJson(request: Request): Promise<unknown> {
  const declaredLength = Number(request.headers.get("Content-Length") ?? "0");
  if (Number.isFinite(declaredLength) && declaredLength > MAX_SESSION_REQUEST_BYTES) {
    throw new SessionRequestError(413, "Session request is too large");
  }
  if (request.body === null) return null;
  const reader = request.body.getReader();
  const chunks: Uint8Array[] = [];
  let length = 0;
  try {
    while (true) {
      const { done, value } = await reader.read();
      if (done) break;
      length += value.length;
      if (length > MAX_SESSION_REQUEST_BYTES) {
        await reader.cancel();
        throw new SessionRequestError(413, "Session request is too large");
      }
      chunks.push(value);
    }
  } finally {
    reader.releaseLock();
  }
  const bytes = new Uint8Array(length);
  let offset = 0;
  for (const chunk of chunks) {
    bytes.set(chunk, offset);
    offset += chunk.length;
  }
  try {
    return JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(bytes)) as unknown;
  } catch {
    throw new SessionRequestError(400, "Invalid session request");
  }
}

function withSecurityHeaders(response: Response, contentSecurityPolicy: string): Response {
  const headers = new Headers(response.headers);
  headers.set("Content-Security-Policy", contentSecurityPolicy);
  headers.set("Cross-Origin-Opener-Policy", "same-origin");
  headers.set("Cross-Origin-Resource-Policy", "same-origin");
  headers.set("Permissions-Policy", "camera=(), geolocation=(), microphone=()");
  headers.set("Referrer-Policy", "no-referrer");
  headers.set("Strict-Transport-Security", "max-age=31536000");
  headers.set("X-Content-Type-Options", "nosniff");
  return new Response(response.body, {
    status: response.status,
    statusText: response.statusText,
    headers,
  });
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isJsonContentType(value: string | null): boolean {
  return value?.split(";", 1)[0]?.trim().toLowerCase() === "application/json";
}
