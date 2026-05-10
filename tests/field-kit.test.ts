import { expect, test } from "bun:test";

import { ExplorationEventLog, type ExplorationEventInput } from "../src/engine/exploration-events.ts";
import { summarizeFieldKit } from "../src/engine/field-kit.ts";

test("field kit summarizes loot-cache events by category and latest find", () => {
  const log = new ExplorationEventLog();

  log.replay([
    loot("travel-pack-cache:0:0", { lootId: "travel-pack-cache" }),
    loot("wetland-reagents:1:0", { lootId: "wetland-reagents" }),
    loot("shrine-offerings:2:0", { lootId: "shrine-offerings" }),
    event("encounter", "mob", "scrib", { role: "mob-trail" }),
    loot("travel-pack-cache:3:0", { lootId: "travel-pack-cache" }),
    loot("berry-bush-forage:4:0", {
      lootId: "berry-bush-forage",
      fieldNote: "Berry bush forage grows along the wet edge of the path.",
    }),
  ]);

  const fieldKit = summarizeFieldKit(log.getSnapshot());

  expect(fieldKit.totalFinds).toBe(5);
  expect(fieldKit.categoryCounts).toEqual({
    supplies: 3,
    reagents: 1,
    relics: 1,
    salvage: 0,
  });
  expect(fieldKit.summaryLabel).toBe("5 field finds");
  expect(fieldKit.lastFindLabel).toBe("Last find: Berry bush forage");
  expect(fieldKit.lastFieldNoteLabel).toBe("Field note: Berry bush forage grows along the wet edge of the path.");
  expect(fieldKit.dominantCategoryLabel).toBe("Mostly supplies");
  expect(fieldKit.entries.map((entry) => [entry.lootId, entry.count, entry.categoryId])).toEqual([
    ["berry-bush-forage", 1, "supplies"],
    ["travel-pack-cache", 2, "supplies"],
    ["wetland-reagents", 1, "reagents"],
    ["shrine-offerings", 1, "relics"],
  ]);
});

test("field kit falls back to subject ids and salvage for unknown caches", () => {
  const log = new ExplorationEventLog();

  log.replay([
    loot("lost-cave-pack:4:-1"),
    loot("weathered-lockbox:5:-1"),
  ]);

  const fieldKit = summarizeFieldKit(log.getSnapshot());

  expect(fieldKit.totalFinds).toBe(2);
  expect(fieldKit.categoryCounts.supplies).toBe(1);
  expect(fieldKit.categoryCounts.salvage).toBe(1);
  expect(fieldKit.entries.map((entry) => [entry.name, entry.categoryLabel])).toEqual([
    ["Lost cave pack", "supplies"],
    ["Weathered lockbox", "salvage"],
  ]);
});

test("empty field kit uses readable empty labels", () => {
  const fieldKit = summarizeFieldKit(new ExplorationEventLog().getSnapshot());

  expect(fieldKit.totalFinds).toBe(0);
  expect(fieldKit.summaryLabel).toBe("Field kit empty");
  expect(fieldKit.lastFindLabel).toBe("No field finds yet");
  expect(fieldKit.lastFieldNoteLabel).toBe("No field note yet");
  expect(fieldKit.dominantCategoryLabel).toBe("No kit pattern yet");
  expect(fieldKit.entries).toEqual([]);
});

function loot(subjectId: string, payload?: ExplorationEventInput["payload"]): ExplorationEventInput {
  return event("use", "object", subjectId, {
    role: "loot-cache",
    payload,
  });
}

function event(
  kind: ExplorationEventInput["kind"],
  subjectType: ExplorationEventInput["subjectType"],
  subjectId: string,
  options: Partial<Omit<ExplorationEventInput, "kind" | "subjectType" | "subjectId">> = {},
): ExplorationEventInput {
  return {
    kind,
    subjectType,
    subjectId,
    name: subjectId,
    ...options,
  };
}
