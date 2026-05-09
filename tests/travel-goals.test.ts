import { expect, test } from "bun:test";

import {
  RouteJournal,
  type TravelGoalDefinition,
} from "../src/engine/travel-goals.ts";

const GOALS: readonly TravelGoalDefinition[] = [
  {
    id: "first-bearings",
    routeId: "pilgrim-road",
    title: "First Bearings",
    journalText: "Read the first road signs.",
    steps: [
      { id: "inspect-causeway", kind: "inspect", targetId: "old_road_causeway", label: "Inspect the old causeway" },
      { id: "read-shrine", kind: "read", targetId: "velothi_shrine", label: "Read the wayshrine" },
      { id: "use-pack", kind: "use", targetId: "ashlander_travel_pack", label: "Use the travel pack", optional: true },
    ],
  },
  {
    id: "ash-road",
    routeId: "ash-road",
    title: "Ash Road",
    journalText: "Follow the ash markers.",
    steps: [
      { id: "visit-ash-marker", kind: "visit", targetId: "ash_marker", label: "Reach an ash marker" },
    ],
  },
];

test("route journal records travel goal progress idempotently", () => {
  const journal = new RouteJournal(GOALS);

  const first = journal.observeProgress({
    routeId: "pilgrim-road",
    kind: "inspect",
    targetId: "old_road_causeway",
  });
  const duplicate = journal.observeProgress({
    routeId: "pilgrim-road",
    kind: "inspect",
    targetId: "old_road_causeway",
  });

  expect(first.changed).toBe(true);
  expect(first.completedStepIds).toEqual(["inspect-causeway"]);
  expect(duplicate.changed).toBe(false);
  expect(journal.getSnapshot().goals[0]).toMatchObject({
    id: "first-bearings",
    status: "active",
    completedStepIds: ["inspect-causeway"],
    requiredStepCount: 2,
    completedRequiredStepCount: 1,
    progress: 0.5,
    completed: false,
  });
});

test("route journal completes goals when all required steps are done", () => {
  const journal = new RouteJournal(GOALS);

  journal.startGoal("first-bearings");
  const optional = journal.observeProgress({
    goalId: "first-bearings",
    kind: "use",
    targetId: "ashlander_travel_pack",
  });
  const completion = journal.observeProgress({
    goalId: "first-bearings",
    kind: "read",
    targetId: "velothi_shrine",
  });
  journal.observeProgress({
    goalId: "first-bearings",
    kind: "inspect",
    targetId: "old_road_causeway",
  });

  expect(optional.completedGoalIds).toEqual([]);
  expect(completion.completedGoalIds).toEqual([]);
  expect(journal.getSnapshot().completedGoalIds).toEqual(["first-bearings"]);
  expect(journal.getSnapshot().goals[0]?.completedStepIds).toEqual([
    "inspect-causeway",
    "read-shrine",
    "use-pack",
  ]);
});

test("route journal export/import preserves progress and sanitizes unknown state", () => {
  const journal = new RouteJournal(GOALS);
  journal.observeProgress({ routeId: "ash-road", kind: "visit", targetId: "ash_marker" });

  const restored = new RouteJournal(GOALS);
  const snapshot = restored.importState({
    ...journal.exportState(),
    goals: [
      ...journal.exportState().goals,
      { id: "unknown-goal", status: "completed", completedStepIds: ["missing"] },
      { id: "first-bearings", status: "completed", completedStepIds: ["read-shrine", "missing-step"] },
    ],
  });

  expect(snapshot.completedGoalIds).toEqual(["ash-road"]);
  expect(snapshot.goals.find((goal) => goal.id === "ash-road")?.completedStepIds).toEqual(["visit-ash-marker"]);
  expect(snapshot.goals.find((goal) => goal.id === "first-bearings")).toMatchObject({
    status: "active",
    completedStepIds: ["read-shrine"],
    completed: false,
  });
  expect(restored.exportState().goals).toEqual([
    { id: "ash-road", status: "completed", completedStepIds: ["visit-ash-marker"] },
    { id: "first-bearings", status: "active", completedStepIds: ["read-shrine"] },
  ]);
});
