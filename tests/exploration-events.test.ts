import { expect, test } from "bun:test";

import {
  buildExplorationEventKey,
  ExplorationEventLog,
  type ExplorationEventInput,
} from "../src/engine/exploration-events.ts";

test("exploration event log records all event kinds with stable sequence ordering", () => {
  const log = new ExplorationEventLog();
  const inputs: ExplorationEventInput[] = [
    event("discover", "biome", "verdant"),
    event("inspect", "landmark", "old_road_causeway", { role: "old-road" }),
    event("read", "landmark", "velothi_shrine", { role: "shrine" }),
    event("use", "route", "pilgrim-road-west", { role: "route-mark" }),
    event("enter-zone", "zone", "ash-road-approach"),
    event("complete-travel-goal", "route", "first-bearings"),
    event("encounter", "npc", "pilgrim-scout"),
  ];

  const result = log.replay(inputs);
  const snapshot = log.getSnapshot();

  expect(result.acceptedEvents.map((record) => record.kind)).toEqual([
    "discover",
    "inspect",
    "read",
    "use",
    "enter-zone",
    "complete-travel-goal",
    "encounter",
  ]);
  expect(snapshot.events.map((record) => record.sequence)).toEqual([1, 2, 3, 4, 5, 6, 7]);
  expect(snapshot.lastEvent?.kind).toBe("encounter");
  expect(snapshot.nextSequence).toBe(8);
  expect(snapshot.events.map((record) => record.key)).toEqual(inputs.map((input) => buildExplorationEventKey(input)));
});

test("exploration event replay is idempotent for once-only keys", () => {
  const log = new ExplorationEventLog();
  const batch = [
    event("discover", "landmark", "silt_shell"),
    event("inspect", "landmark", "silt_shell"),
    event("read", "landmark", "silt_shell"),
  ];

  const first = log.replay(batch);
  const second = log.replay(batch);

  expect(first.acceptedEvents).toHaveLength(3);
  expect(second.acceptedEvents).toEqual([]);
  expect(second.duplicateKeys).toEqual(batch.map((input) => buildExplorationEventKey(input)));
  expect(log.getSnapshot().events.map((record) => record.sequence)).toEqual([1, 2, 3]);
});

test("repeatable events accept distinct occurrence keys but keep first-use awards once-only", () => {
  const log = new ExplorationEventLog();

  const first = log.record(event("use", "object", "ashlander_travel_pack", {
    occurrenceId: "rest-1",
    repeatable: true,
  }));
  const duplicate = log.record(event("use", "object", "ashlander_travel_pack", {
    occurrenceId: "rest-1",
    repeatable: true,
  }));
  const second = log.record(event("use", "object", "ashlander_travel_pack", {
    occurrenceId: "rest-2",
    repeatable: true,
  }));

  expect(first.accepted).toBe(true);
  expect(duplicate.accepted).toBe(false);
  expect(second.accepted).toBe(true);
  expect(log.getSnapshot().events.map((record) => record.key)).toEqual([
    buildExplorationEventKey({
      kind: "use",
      subjectType: "object",
      subjectId: "ashlander_travel_pack",
      role: "default",
      occurrenceId: "rest-1",
    }),
    buildExplorationEventKey({
      kind: "use",
      subjectType: "object",
      subjectId: "ashlander_travel_pack",
      role: "default",
      occurrenceId: "rest-2",
    }),
  ]);
  expect(log.getSnapshot().events.map((record) => record.skillAwards.length)).toEqual([1, 0]);
  expect(log.getSnapshot().awardedSkillKeys).toEqual(["use:object:ashlander_travel_pack"]);
});

test("first read and route use events carry pure skill award metadata", () => {
  const log = new ExplorationEventLog();

  log.replay([
    event("read", "landmark", "velothi_shrine", { role: "shrine" }),
    event("use", "route", "pilgrim-road-west", { role: "route-mark" }),
  ]);
  const [read, use] = log.getSnapshot().events;

  expect(read?.skillAwards).toEqual([{
    skillId: "lore",
    xp: 25,
    reason: "First read",
    awardKey: "read:landmark:velothi_shrine",
    onceOnly: true,
  }]);
  expect(use?.skillAwards).toEqual([{
    skillId: "cartography",
    xp: 20,
    reason: "First use",
    awardKey: "use:route:pilgrim-road-west",
    onceOnly: true,
  }]);
});

test("event export and import round-trip preserves order and advances sequence", () => {
  const log = new ExplorationEventLog();
  log.replay([
    event("discover", "underground", "rooted", { worldPosition: [1, 2, 3] }),
    event("read", "landmark", "old_road_causeway", {
      flavorText: "Raised stones cross the low ground.",
      payload: { routeId: "pilgrim-road", unknownNested: { retained: true } },
    }),
  ]);

  const restored = new ExplorationEventLog();
  const importResult = restored.importState({
    ...log.exportState(),
    events: [...log.exportState().events].reverse(),
  });
  restored.record(event("encounter", "mob", "scrib"));

  const snapshot = restored.getSnapshot();
  expect(importResult.importedEvents).toBe(2);
  expect(importResult.ignoredInvalidEvents).toBe(0);
  expect(snapshot.events.map((record) => record.sequence)).toEqual([1, 2, 3]);
  expect(snapshot.events.map((record) => record.kind)).toEqual(["discover", "read", "encounter"]);
  expect(snapshot.events[0]?.worldPosition).toEqual([1, 2, 3]);
  expect(snapshot.events[1]?.payload).toEqual({ routeId: "pilgrim-road", unknownNested: { retained: true } });
});

test("import ignores unknown event kinds, invalid events, and duplicate keys", () => {
  const known = {
    version: 1,
    key: "discover:biome:verdant:default",
    kind: "discover",
    subjectId: "verdant",
    subjectType: "biome",
    role: "default",
    name: "Verdant Reach",
    flavorText: null,
    sequence: 1,
    repeatable: false,
    skillAwards: [],
    ignoredTopLevelField: "dropped",
  };
  const log = new ExplorationEventLog();

  const result = log.importState({
    version: 1,
    nextSequence: 4,
    awardedSkillKeys: [],
    events: [
      known,
      { ...known, sequence: 2 },
      { ...known, key: "future:kind", kind: "trade", sequence: 3 },
      { ...known, key: "bad:subject", subjectType: "inventory", sequence: 4 },
      { ...known, key: "bad:sequence", sequence: 0 },
    ] as never,
  });
  const exported = log.exportState();

  expect(result.importedEvents).toBe(1);
  expect(result.ignoredDuplicateKeys).toEqual(["discover:biome:verdant:default"]);
  expect(result.ignoredUnknownKinds).toBe(1);
  expect(result.ignoredInvalidEvents).toBe(2);
  expect(exported.events).toEqual([{
    version: 1,
    key: "discover:biome:verdant:default",
    kind: "discover",
    subjectId: "verdant",
    subjectType: "biome",
    role: "default",
    name: "Verdant Reach",
    flavorText: null,
    sequence: 1,
    repeatable: false,
    skillAwards: [],
  }]);
  expect(exported.nextSequence).toBe(4);
});

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
