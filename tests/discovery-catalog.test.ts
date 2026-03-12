import { expect, test } from "bun:test";

import {
  describeDiscovery,
  formatDiscoveryInline,
  formatDiscoveryLabel,
} from "../src/engine/discovery-catalog.ts";

test("discovery catalog exposes player-facing names with identifiers", () => {
  expect(describeDiscovery("biome", "verdant")).toEqual({
    category: "biome",
    categoryLabel: "Biome",
    id: "verdant",
    name: "Verdant Reach",
    inlineLabel: "Verdant Reach [verdant]",
    fullLabel: "Biome: Verdant Reach [verdant]",
  });
  expect(formatDiscoveryInline("regional-variant", "marsh_blackwater")).toBe(
    "Blackwater Channel [marsh_blackwater]",
  );
  expect(formatDiscoveryLabel("landmark", "redwood")).toBe(
    "Landmark: Skyroot Redwood [redwood]",
  );
});

test("discovery catalog keeps sensible fallbacks for unknown ids", () => {
  expect(formatDiscoveryInline("landmark", null)).toBe("None");
  expect(describeDiscovery("landmark", "strange_object").name).toBe("Strange Object");
});
