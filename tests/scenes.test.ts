import { expect, test } from "bun:test";

import { getStressSceneDefinitions } from "../src/engine/scenes.ts";

test("stress suite exposes targeted performance workloads", () => {
  const definitions = getStressSceneDefinitions();
  const ids = definitions.map((definition) => definition.id);

  expect(definitions.length).toBeGreaterThanOrEqual(4);
  expect(new Set(ids).size).toBe(ids.length);
  expect(definitions.every((definition) => definition.kind === "performance" && definition.stress)).toBe(true);
  for (const expectedId of ["editStorm256", "stressDrawCalls512", "stressMicroCubes256", "stressScreens256"]) {
    expect(ids.includes(expectedId)).toBe(true);
  }
});
