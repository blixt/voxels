import { expect, test } from "bun:test";

import { ExplorationEventLog, type ExplorationEventInput } from "../src/engine/exploration-events.ts";
import { getLootJournalCandidateState, summarizeLootJournal } from "../src/engine/loot-journal.ts";

test("loot journal records first collect state for a cache", () => {
  const log = new ExplorationEventLog();

  log.record(loot("berry-bush-forage:forage-patch:4:0", {
    lootId: "berry-bush-forage",
    categoryId: "supplies",
    fieldNote: "Ripe berries sit just under the leaves.",
  }));

  const journal = summarizeLootJournal(log.getSnapshot());

  expect(journal.totalCollectedCaches).toBe(1);
  expect(journal.totalRevisitedCaches).toBe(0);
  expect(journal.totalCollectEvents).toBe(1);
  expect(journal.totalRevisitEvents).toBe(0);
  expect(journal.entries).toEqual([{
    subjectId: "berry-bush-forage:forage-patch:4:0",
    lootId: "berry-bush-forage",
    categoryId: "supplies",
    collected: true,
    revisited: false,
    collectCount: 1,
    revisitCount: 0,
    eventCount: 1,
    firstSequence: 1,
    lastSequence: 1,
    lastNote: "Ripe berries sit just under the leaves.",
  }]);
});

test("loot journal marks repeated cache events as revisits", () => {
  const log = new ExplorationEventLog();

  log.replay([
    loot("travel-pack-cache:supply-cache:1:0", {
      lootId: "travel-pack-cache",
      fieldNote: "A dry bundle is tucked behind stone.",
    }, "first"),
    loot("travel-pack-cache:supply-cache:1:0", {
      lootId: "travel-pack-cache",
      fieldNote: "Only strap marks remain after the second check.",
    }, "second"),
    loot("wetland-reagents:reagent-patch:2:0", {
      lootId: "wetland-reagents",
      fieldNote: "Spore caps bead with rain.",
    }),
  ]);

  const journal = summarizeLootJournal(log.getSnapshot());
  const revisited = getLootJournalCandidateState(journal, {
    subjectId: "travel-pack-cache:supply-cache:1:0",
    lootId: "travel-pack-cache",
  });

  expect(journal.totalCollectedCaches).toBe(2);
  expect(journal.totalRevisitedCaches).toBe(1);
  expect(journal.totalCollectEvents).toBe(2);
  expect(journal.totalRevisitEvents).toBe(1);
  expect(revisited).toMatchObject({
    collected: true,
    revisited: true,
    collectCount: 1,
    revisitCount: 1,
    eventCount: 2,
    lastNote: "Only strap marks remain after the second check.",
    matchedSubjectId: "travel-pack-cache:supply-cache:1:0",
    match: "subject",
  });
});

test("loot journal candidate lookup falls back through loot id and category", () => {
  const log = new ExplorationEventLog();

  log.replay([
    loot("subject-without-payload-loot:3:0", {
      categoryId: "reagents",
      fieldNote: "A reagent pouch has already been disturbed.",
    }),
    loot("opaque-cache-subject", {
      lootId: "shrine-offerings",
      categoryId: "relics",
      fieldNote: "Offerings are arranged in a careful ring.",
    }),
  ]);

  const journal = summarizeLootJournal(log.getSnapshot());
  const subjectFallback = getLootJournalCandidateState(journal, {
    subjectId: "subject-without-payload-loot:3:0",
  });
  const lootFallback = getLootJournalCandidateState(journal, {
    subjectId: "different-shrine-subject",
    lootId: "shrine-offerings",
  });
  const categoryFallback = getLootJournalCandidateState(journal, {
    subjectId: "unknown-reagent-site",
    lootId: "unknown-reagent-site",
    categoryId: "reagents",
  });

  expect(subjectFallback).toMatchObject({
    lootId: "subject-without-payload-loot",
    collected: true,
    match: "subject",
  });
  expect(lootFallback).toMatchObject({
    collected: true,
    match: "loot-id",
    matchedSubjectId: "opaque-cache-subject",
    lastNote: "Offerings are arranged in a careful ring.",
  });
  expect(categoryFallback).toMatchObject({
    collected: true,
    match: "category",
    matchedSubjectId: "subject-without-payload-loot:3:0",
    lastNote: "A reagent pouch has already been disturbed.",
  });
});

test("loot journal recognizes anchored plant forage revisits", () => {
  const log = new ExplorationEventLog();

  log.replay([
    loot("berry-bush-forage:berry_bush:4:0", {
      lootId: "berry-bush-forage",
      categoryId: "vegetation-forage",
      sourceLandmarkId: "berry_bush",
      anchoredToVisibleLandmark: true,
      fieldNote: "Berry Bush Forage is visible on the berry bush.",
    }),
    loot("berry-bush-forage:berry_bush:4:0", {
      lootId: "berry-bush-forage",
      categoryId: "vegetation-forage",
      sourceLandmarkId: "berry_bush",
      anchoredToVisibleLandmark: true,
      fieldNote: "Only stripped stems remain.",
    }, "revisit-2"),
  ]);

  const state = getLootJournalCandidateState(log.getSnapshot(), {
    subjectId: "berry-bush-forage:berry_bush:4:0",
    lootId: "berry-bush-forage",
    categoryId: "vegetation-forage",
  });
  const similarPlant = getLootJournalCandidateState(log.getSnapshot(), {
    subjectId: "berry-bush-forage:berry_bush:5:0",
    lootId: "berry-bush-forage",
    categoryId: "vegetation-forage",
  });

  expect(state).toMatchObject({
    collected: true,
    revisited: true,
    eventCount: 2,
    match: "subject",
    lastNote: "Only stripped stems remain.",
  });
  expect(similarPlant).toMatchObject({
    collected: true,
    match: "loot-id",
    matchedSubjectId: "berry-bush-forage:berry_bush:4:0",
  });
});

test("empty loot journal returns empty state and candidate misses", () => {
  const log = new ExplorationEventLog();
  log.record({
    kind: "encounter",
    subjectType: "mob",
    subjectId: "scrib",
    role: "mob-trail",
    name: "Scrib trail",
  });

  const journal = summarizeLootJournal(log.getSnapshot());
  const candidate = getLootJournalCandidateState(journal, {
    subjectId: "trail-forage:forage-patch:0:0",
    lootId: "trail-forage",
  });

  expect(journal).toEqual({
    totalCollectedCaches: 0,
    totalRevisitedCaches: 0,
    totalCollectEvents: 0,
    totalRevisitEvents: 0,
    entries: [],
  });
  expect(candidate).toEqual({
    subjectId: "trail-forage:forage-patch:0:0",
    lootId: "trail-forage",
    categoryId: null,
    collected: false,
    revisited: false,
    collectCount: 0,
    revisitCount: 0,
    eventCount: 0,
    lastNote: null,
    matchedSubjectId: null,
    match: "none",
  });
});

function loot(
  subjectId: string,
  payload?: ExplorationEventInput["payload"],
  occurrenceId?: string,
): ExplorationEventInput {
  return {
    kind: "use",
    subjectType: "object",
    subjectId,
    role: "loot-cache",
    name: subjectId,
    repeatable: Boolean(occurrenceId),
    occurrenceId,
    payload,
  };
}
