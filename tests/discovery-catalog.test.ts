import { expect, test } from "bun:test";

import {
  describeDiscovery,
  formatDiscoveryInline,
  formatDiscoveryLabel,
  formatDiscoveryName,
} from "../src/engine/discovery-catalog.ts";

test("discovery catalog exposes player-facing names with identifiers", () => {
  expect(describeDiscovery("biome", "verdant")).toEqual({
    category: "biome",
    categoryLabel: "Biome",
    id: "verdant",
    name: "Verdant Reach",
    role: "region",
    roleLabel: "Region",
    flavorText: null,
    progressionHint: "Travel and region discovery train Cartography.",
    inlineLabel: "Verdant Reach [verdant]",
    fullLabel: "Biome: Verdant Reach [verdant]",
  });
  expect(formatDiscoveryInline("regional-variant", "marsh_blackwater")).toBe(
    "Blackwater Channel [marsh_blackwater]",
  );
  expect(formatDiscoveryLabel("landmark", "redwood")).toBe(
    "Landmark: Skyroot Redwood [redwood]",
  );
  expect(describeDiscovery("landmark", "velothi_shrine").flavorText).toBe(
    "A small shrine watches the road with worn amber light.",
  );
  expect(describeDiscovery("landmark", "velothi_shrine").roleLabel).toBe("Shrine");
  expect(formatDiscoveryName("landmark", "velothi_shrine")).toBe("Velothi Wayshrine");
});

test("discovery catalog keeps sensible fallbacks for unknown ids", () => {
  expect(formatDiscoveryInline("landmark", null)).toBe("None");
  expect(describeDiscovery("landmark", "strange_object").name).toBe("Strange Object");
  expect(formatDiscoveryName("biome", null)).toBe("Unknown");
});

test("discovery catalog gives old-road landmarks journal flavor and progression hints", () => {
  const ashMarker = describeDiscovery("landmark", "ash_marker");
  const causeway = describeDiscovery("landmark", "old_road_causeway");
  const boneChimes = describeDiscovery("landmark", "bone_chimes");
  const travelPack = describeDiscovery("landmark", "ashlander_travel_pack");
  const ziggurat = describeDiscovery("landmark", "velothi_ziggurat");

  expect(ashMarker.role).toBe("old-road");
  expect(ashMarker.roleLabel).toBe("Old Road");
  expect(ashMarker.flavorText).toContain("volcanic country");
  expect(ashMarker.progressionHint).toBe("Landmark discovery trains Naturalist.");
  expect(causeway.name).toBe("Old Road Causeway");
  expect(causeway.role).toBe("old-road");
  expect(boneChimes.name).toBe("Bone Wind Chimes");
  expect(boneChimes.role).toBe("old-road");
  expect(boneChimes.flavorText).toContain("pale bones");
  expect(travelPack.name).toBe("Ashlander Travel Pack");
  expect(travelPack.role).toBe("old-road");
  expect(travelPack.flavorText).toContain("bedroll");
  expect(ziggurat.roleLabel).toBe("Shrine");
});
