import { get as httpGet } from "node:http";

/** Reads the bounded development-instance identity published by the native daemon. */
export function worldServiceHealthNonce(host: string, port: number): Promise<string | null> {
  return new Promise((resolve) => {
    const request = httpGet(
      {
        hostname: host,
        port,
        path: "/healthz",
        timeout: 250,
      },
      (response) => {
        if (response.statusCode !== 200) {
          response.resume();
          resolve(null);
          return;
        }
        let body = "";
        response.setEncoding("utf8");
        response.on("data", (chunk: string) => {
          if (body.length <= 256) body += chunk;
        });
        response.once("end", () => resolve(body.length <= 256 ? body : null));
      },
    );
    request.once("timeout", () => {
      request.destroy();
      resolve(null);
    });
    request.once("error", () => resolve(null));
  });
}
