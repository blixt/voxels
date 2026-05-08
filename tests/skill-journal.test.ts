import { expect, test } from "bun:test";

import { SkillJournal } from "../src/engine/skill-journal.ts";
import type { DiscoveryEvent } from "../src/engine/exploration-journal.ts";

test("skill journal awards deterministic exploration XP once per discovery sequence", () => {
  const journal = new SkillJournal();
  const discoveries = [
    event(1, "biome", "verdant"),
    event(2, "regional-variant", "verdant_karst"),
    event(3, "landmark", "redwood"),
    event(4, "underground", "rooted"),
  ];

  const first = journal.observeDiscoveries(discoveries);
  const second = journal.observeDiscoveries(discoveries);

  expect(first.skills.find((skill) => skill.id === "cartography")?.totalXp).toBe(55);
  expect(first.skills.find((skill) => skill.id === "lore")?.totalXp).toBe(120);
  expect(first.skills.find((skill) => skill.id === "naturalist")?.totalXp).toBe(35);
  expect(first.skills.find((skill) => skill.id === "spelunking")?.totalXp).toBe(90);
  expect(second).toEqual(first);
  expect(second.lastProcessedDiscoverySequence).toBe(4);
});

test("skill journal levels and focus skill advance from repeated usage", () => {
  const journal = new SkillJournal();
  const discoveries = Array.from({ length: 10 }, (_, index) => event(index + 1, "landmark", `landmark_${index}`));

  const snapshot = journal.observeDiscoveries(discoveries);
  const naturalist = snapshot.skills.find((skill) => skill.id === "naturalist")!;

  expect(naturalist.totalXp).toBe(350);
  expect(naturalist.level).toBeGreaterThan(2);
  expect(naturalist.progressRatio).toBeGreaterThanOrEqual(0);
  expect(naturalist.progressRatio).toBeLessThan(1);
  expect(snapshot.focusSkill.id).toBe("naturalist");
  expect(snapshot.totalLevel).toBeGreaterThan(snapshot.skills.length);
});

test("skill journal awards usage XP from surface and underground travel", () => {
  const journal = new SkillJournal();

  journal.observeTravel(48, "surface");
  journal.observeTravel(48, "underground");
  const snapshot = journal.getSnapshot();

  expect(snapshot.skills.find((skill) => skill.id === "cartography")?.totalXp).toBe(2);
  expect(snapshot.skills.find((skill) => skill.id === "spelunking")?.totalXp).toBe(2);
  expect(snapshot.travelMeters).toBe(96);
});

test("skill journal rewards route and shrine discoveries as RPG progression hooks", () => {
  const journal = new SkillJournal();
  const discoveries = [
    event(1, "landmark", "old_road_causeway"),
    event(2, "landmark", "velothi_shrine"),
  ];

  const first = journal.observeDiscoveries(discoveries);
  const second = journal.observeDiscoveries(discoveries);

  expect(first.skills.find((skill) => skill.id === "cartography")?.totalXp).toBe(35);
  expect(first.skills.find((skill) => skill.id === "lore")?.totalXp).toBe(75);
  expect(first.skills.find((skill) => skill.id === "naturalist")?.totalXp).toBe(70);
  expect(second).toEqual(first);
  expect(second.lastProcessedDiscoverySequence).toBe(2);
});

test("skill journal reset clears XP and processed discovery state", () => {
  const journal = new SkillJournal();
  journal.observeDiscoveries([event(1, "biome", "verdant")]);
  journal.reset();

  const snapshot = journal.getSnapshot();

  expect(snapshot.skills.map((skill) => skill.totalXp)).toEqual([0, 0, 0, 0]);
  expect(snapshot.skills.map((skill) => skill.level)).toEqual([1, 1, 1, 1]);
  expect(snapshot.lastProcessedDiscoverySequence).toBe(0);
});

test("skill journal exports and imports XP state", () => {
  const journal = new SkillJournal();
  journal.observeDiscoveries([
    event(1, "biome", "verdant"),
    event(2, "landmark", "redwood"),
    event(3, "landmark", "oak"),
  ]);

  const restored = new SkillJournal();
  restored.importState(journal.exportState());
  const snapshot = restored.getSnapshot();

  expect(snapshot.skills.find((skill) => skill.id === "cartography")?.totalXp).toBe(55);
  expect(snapshot.skills.find((skill) => skill.id === "naturalist")?.totalXp).toBe(70);
  expect(snapshot.lastProcessedDiscoverySequence).toBe(3);
  expect(restored.observeDiscoveries([event(2, "landmark", "redwood")])).toEqual(snapshot);
});

test("skill journal preserves travel remainder through export and import", () => {
  const journal = new SkillJournal();
  journal.observeTravel(23, "surface");

  const restored = new SkillJournal();
  restored.importState(journal.exportState());
  restored.observeTravel(1, "surface");
  const cartography = restored.getSnapshot().skills.find((skill) => skill.id === "cartography")!;

  expect(cartography.totalXp).toBe(1);
  expect(restored.getSnapshot().travelMeters).toBe(24);
});

function event(sequence: number, category: DiscoveryEvent["category"], id: string): DiscoveryEvent {
  return {
    category,
    id,
    name: id,
    flavorText: null,
    identifier: id,
    categoryLabel: category,
    label: `${category}:${id}`,
    sequence,
  };
}
