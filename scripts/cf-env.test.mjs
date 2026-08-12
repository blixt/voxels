import { describe, expect, it } from "vite-plus/test";
import { stripAmbientCloudflareAuth } from "./cf-env.mjs";

describe("repository-local Cloudflare environment", () => {
  it("strips current and deprecated ambient account selectors unless explicitly inherited", () => {
    const ambient = {
      CF_ACCOUNT_ID: "deprecated-account",
      CF_API_TOKEN: "deprecated-token",
      CLOUDFLARE_ACCOUNT_ID: "current-account",
      CLOUDFLARE_API_TOKEN: "current-token",
      UNRELATED: "preserved",
    };

    expect(stripAmbientCloudflareAuth(ambient, false)).toEqual({ UNRELATED: "preserved" });
    expect(stripAmbientCloudflareAuth(ambient, true)).toEqual(ambient);
  });
});
