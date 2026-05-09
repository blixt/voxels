import { expect, test } from "bun:test";

import {
  buildExplorationInteractionEventInput,
  resolveExplorationInteractionTarget,
  type ExplorationInteractionCandidate,
} from "../src/engine/exploration-interactions.ts";

test("interaction resolver chooses the best facing target and builds inspect/read/use prompts", () => {
  const candidates: ExplorationInteractionCandidate[] = [
    {
      id: "velothi_shrine",
      subjectType: "landmark",
      name: "Velothi Wayshrine",
      role: "shrine",
      worldPosition: [0, 0, 3],
      prompts: [
        "inspect",
        { verb: "read", label: "Read shrine etching", description: "Faded pilgrim marks." },
        "use",
      ],
      flavorText: "A small shrine watches the road.",
      payload: { routeId: "pilgrim-road" },
    },
    {
      id: "ashlander_travel_pack",
      subjectType: "object",
      name: "Ashlander Travel Pack",
      worldPosition: [0, 0, -2],
      prompts: ["inspect", "use"],
    },
  ];

  const resolution = resolveExplorationInteractionTarget({
    viewerPosition: [0, 0, 0],
    viewerForward: [0, 0, 1],
    candidates,
  });

  expect(resolution.target?.id).toBe("velothi_shrine");
  expect(resolution.candidates.map((candidate) => candidate.id)).toEqual([
    "velothi_shrine",
    "ashlander_travel_pack",
  ]);
  expect(resolution.target?.prompts.map((prompt) => prompt.label)).toEqual([
    "Inspect Velothi Wayshrine",
    "Read shrine etching",
    "Use Velothi Wayshrine",
  ]);
  expect(resolution.target?.prompts[1]?.eventInput).toEqual({
    kind: "read",
    subjectType: "landmark",
    subjectId: "velothi_shrine",
    role: "shrine",
    name: "Velothi Wayshrine",
    flavorText: "A small shrine watches the road.",
    worldPosition: [0, 0, 3],
    payload: { routeId: "pilgrim-road" },
  });
});

test("interaction resolver filters unreachable or promptless targets", () => {
  const resolution = resolveExplorationInteractionTarget({
    viewerPosition: [0, 0, 0],
    maxDistanceMeters: 5,
    candidates: [
      {
        id: "near",
        subjectType: "object",
        worldPosition: [1, 0, 1],
        prompts: ["inspect"],
      },
      {
        id: "far",
        subjectType: "object",
        worldPosition: [0, 0, 8],
        interactionRadiusMeters: 12,
        prompts: ["inspect"],
      },
      {
        id: "silent",
        subjectType: "object",
        worldPosition: [0, 0, 2],
        prompts: [],
      },
    ],
  });

  expect(resolution.candidates.map((candidate) => candidate.id)).toEqual(["near"]);
  expect(resolution.target?.distanceMeters).toBeCloseTo(Math.sqrt(2), 8);
});

test("interaction event helper keeps resolved target identity stable", () => {
  const eventInput = buildExplorationInteractionEventInput(
    {
      id: "old_road_causeway",
      subjectType: "landmark",
      name: "Old Road Causeway",
      role: "old-road",
      worldPosition: [4, 2, 1],
    },
    "inspect",
    { flavorText: "Raised stones cross the low ground." },
  );

  expect(eventInput).toEqual({
    kind: "inspect",
    subjectType: "landmark",
    subjectId: "old_road_causeway",
    role: "old-road",
    name: "Old Road Causeway",
    worldPosition: [4, 2, 1],
    flavorText: "Raised stones cross the low ground.",
  });
});
