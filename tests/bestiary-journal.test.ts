import { expect, test } from "bun:test";

import { summarizeBestiary } from "../src/engine/bestiary-journal.ts";
import { ExplorationEventLog, type ExplorationEventInput } from "../src/engine/exploration-events.ts";

test("bestiary summarizes mob signs by faction and latest field note", () => {
  const log = new ExplorationEventLog();

  log.replay([
    mob("ashlander-scouts", "mob-spoor", {
      factionId: "ashlander-scouts",
      moodId: "ash-wind-watch",
      fieldNote: "Ashlander sign is recent but scattered.",
    }),
    mob("kwama-brood:mob-lair:1:2", "mob-lair", {
      factionId: "kwama-brood",
      moodId: "cave-threshold",
      fieldNote: "Kwama Brood sign is fresh and crowded.",
    }),
    mob("ashlander-scouts:mob-nest:2:3", "mob-nest", {
      factionId: "ashlander-scouts",
      moodId: "ash-wind-watch",
      fieldNote: "Ashlander Scouts have marked a windbreak camp.",
    }),
  ]);

  const bestiary = summarizeBestiary(log.getSnapshot());

  expect(bestiary.totalSightings).toBe(3);
  expect(bestiary.entryCount).toBe(2);
  expect(bestiary.summaryLabel).toBe("3 mob sightings");
  expect(bestiary.lastSightingLabel).toBe("Last sign: Ashlander Scouts");
  expect(bestiary.lastFieldNoteLabel).toBe("Mob note: Ashlander Scouts have marked a windbreak camp.");
  expect(bestiary.dominantFactionLabel).toBe("Most signs: Ashlander Scouts");
  expect(bestiary.entries.map((entry) => [entry.id, entry.count, entry.lastRole])).toEqual([
    ["ashlander-scouts", 2, "mob-nest"],
    ["kwama-brood", 1, "mob-lair"],
  ]);
});

test("empty bestiary uses readable empty labels", () => {
  const bestiary = summarizeBestiary(new ExplorationEventLog().getSnapshot());

  expect(bestiary.totalSightings).toBe(0);
  expect(bestiary.entryCount).toBe(0);
  expect(bestiary.summaryLabel).toBe("Bestiary empty");
  expect(bestiary.lastSightingLabel).toBe("No mob signs yet");
  expect(bestiary.lastFieldNoteLabel).toBe("No mob field note yet");
  expect(bestiary.dominantFactionLabel).toBe("No dominant mob sign");
});

function mob(
  subjectId: string,
  role: string,
  payload: Record<string, string>,
): ExplorationEventInput {
  return {
    kind: "encounter",
    subjectType: "mob",
    subjectId,
    role,
    name: subjectId,
    flavorText: payload.fieldNote ?? null,
    payload,
  };
}
