import { expect, test } from "bun:test";

import { describeInteractionSkillGates } from "../src/engine/interaction-skill-gates.ts";

test("interaction skill gates describe locked low-level decisions", () => {
  const gates = describeInteractionSkillGates({
    cartographyLevel: 1,
    naturalistLevel: 1,
    spelunkingLevel: 1,
    loreLevel: 1,
  });

  expect(gates).toMatchObject({
    canReadRoadMarks: false,
    canIdentifyForageQuality: false,
    canAssessCaveRoute: false,
    canInterpretLore: false,
  });
  expect(gates.roadMarkHint).toContain("Cartography 2");
  expect(gates.forageHint).toContain("Naturalist 2");
  expect(gates.caveRouteHint).toContain("Spelunking 2");
  expect(gates.loreHint).toContain("Lore 2");
});

test("interaction skill gates unlock richer decision hints at level two", () => {
  const gates = describeInteractionSkillGates({
    cartographyLevel: 2,
    naturalistLevel: 3,
    spelunkingLevel: 2,
    loreLevel: 4,
  });

  expect(gates).toMatchObject({
    canReadRoadMarks: true,
    canIdentifyForageQuality: true,
    canAssessCaveRoute: true,
    canInterpretLore: true,
  });
  expect(gates.roadMarkHint).toContain("reads");
  expect(gates.forageHint).toContain("safest cut");
  expect(gates.caveRouteHint).toContain("return path");
  expect(gates.loreHint).toContain("pilgrim meaning");
});
