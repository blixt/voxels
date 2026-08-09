import { get as httpGet } from "node:http";

const HEALTH_RESPONSE_MAX_BYTES = 256;
const HEALTH_DEADLINE_MS = 250;

/** Reads the bounded development-instance identity published by the native daemon. */
export function worldServiceHealthNonce(host: string, port: number): Promise<string | null> {
  return new Promise((resolve) => {
    let settled = false;
    let deadline: NodeJS.Timeout | undefined;
    let request: ReturnType<typeof httpGet> | undefined;
    const finish = (value: string | null): void => {
      if (settled) return;
      settled = true;
      if (deadline !== undefined) clearTimeout(deadline);
      request?.destroy();
      resolve(value);
    };
    request = httpGet(
      {
        hostname: host,
        port,
        path: "/healthz",
      },
      (response) => {
        if (response.statusCode !== 200) {
          finish(null);
          return;
        }
        const chunks: Buffer[] = [];
        let length = 0;
        response.on("data", (chunk: Buffer) => {
          length += chunk.length;
          if (length > HEALTH_RESPONSE_MAX_BYTES) {
            response.destroy();
            finish(null);
            return;
          }
          chunks.push(chunk);
        });
        response.once("end", () => finish(Buffer.concat(chunks, length).toString("utf8")));
        response.once("error", () => finish(null));
      },
    );
    request.once("error", () => finish(null));
    deadline = setTimeout(() => finish(null), HEALTH_DEADLINE_MS);
  });
}
