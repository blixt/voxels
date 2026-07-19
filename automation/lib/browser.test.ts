import { describe, expect, it } from "vite-plus/test";
import { chromeWebGpuLaunchOptions, isBrowserConsoleFailure } from "./browser.ts";

describe("browser automation defaults", () => {
  it("always rejects console errors and filters warnings narrowly", () => {
    const warnings = /webgpu|sqlite/iu;
    expect(isBrowserConsoleFailure("error", "render loop stopped", warnings)).toBe(true);
    expect(isBrowserConsoleFailure("warning", "WebGPU validation", warnings)).toBe(true);
    expect(isBrowserConsoleFailure("warning", "development hint", warnings)).toBe(false);
    expect(isBrowserConsoleFailure("log", "sqlite", warnings)).toBe(false);
  });

  it("pre-authorizes the hermetic loopback daemon in headless Chrome", () => {
    expect(chromeWebGpuLaunchOptions().args).toContain(
      "--disable-features=LocalNetworkAccessChecks",
    );
  });

  it("uses normal WebGPU validation rather than unsafe shader extensions", () => {
    expect(chromeWebGpuLaunchOptions().args).not.toContain("--enable-unsafe-webgpu");
    expect(chromeWebGpuLaunchOptions().args).not.toContain("--enable-features=WebGPU");
  });
});
