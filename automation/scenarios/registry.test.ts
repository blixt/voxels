import { describe, expect, it } from "vite-plus/test";
import { loadScenarios, scenarioById } from "./registry.ts";

describe("automation scenario discovery", () => {
  it("discovers every scenario without manual registry edits", async () => {
    const scenarios = await loadScenarios();
    const ids = scenarios.map((scenario) => scenario.id);
    expect(ids).toEqual(ids.toSorted());
    expect(new Set(ids).size).toBe(ids.length);
    expect(ids).toContain("bot-load");
    expect(ids).toContain("lod-transition");
    expect(ids).toContain("render-profile");
    expect(await scenarioById("world-lab")).toBeDefined();
  });
});
