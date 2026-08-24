import { describe, expect, it } from "vite-plus/test";
import { stripAmbientCloudflareAuth } from "./cf-env.mjs";

describe("repository-local Cloudflare environment", () => {
  it("strips ambient Cloudflare credentials and target selectors unless explicitly inherited", () => {
    const ambient = {
      CF_ACCOUNT_ID: "deprecated-account",
      CF_API_BASE_URL: "https://deprecated-api.invalid/client/v4",
      CF_API_TOKEN: "deprecated-token",
      CLOUDFLARE_ACCOUNT_ID: "current-account",
      CLOUDFLARE_API_BASE_URL: "https://current-api.invalid/client/v4",
      CLOUDFLARE_API_TOKEN: "current-token",
      CLOUDFLARE_COMPLIANCE_REGION: "fedramp_high",
      CLOUDFLARE_ENV: "ambient-environment",
      WRANGLER_API_ENVIRONMENT: "staging",
      UNRELATED: "preserved",
    };

    expect(stripAmbientCloudflareAuth(ambient, false)).toEqual({ UNRELATED: "preserved" });
    expect(stripAmbientCloudflareAuth(ambient, true)).toEqual(ambient);
  });
});
