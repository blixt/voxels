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
      skillAwards: [{
        skillId: "lore",
        xp: 25,
        reason: "Shrine reading",
        awardKey: "read:landmark:velothi_shrine",
        onceOnly: true,
      }],
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
    skillAwards: [{
      skillId: "lore",
      xp: 25,
      reason: "Shrine reading",
      awardKey: "read:landmark:velothi_shrine",
      onceOnly: true,
    }],
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

test("interaction resolver lets authored landmarks outrank mob trails and loot caches", () => {
  const resolution = resolveExplorationInteractionTarget({
    viewerPosition: [0, 0, 0],
    viewerForward: [0, 0, 1],
    candidates: [
      {
        id: "trail-forage:0:0",
        subjectType: "object",
        name: "Trail Forage",
        role: "loot-cache",
        priority: 1,
        worldPosition: [0, 0, 1],
        prompts: ["use"],
      },
      {
        id: "kwama-brood:0:0",
        subjectType: "mob",
        name: "Kwama Brood",
        role: "mob-trail",
        priority: 8,
        worldPosition: [0, 0, 1.2],
        prompts: ["inspect"],
      },
      {
        id: "old_road_causeway",
        subjectType: "landmark",
        name: "Old Road Causeway",
        role: "old-road",
        priority: 20,
        worldPosition: [0, 0, 1.4],
        prompts: ["inspect"],
      },
    ],
  });

  expect(resolution.candidates.map((candidate) => candidate.id)).toEqual([
    "old_road_causeway",
    "kwama-brood:0:0",
    "trail-forage:0:0",
  ]);
  expect(resolution.target?.subjectType).toBe("landmark");
});

test("interaction resolver surfaces mob trails ahead of local loot when no landmark is present", () => {
  const resolution = resolveExplorationInteractionTarget({
    viewerPosition: [0, 0, 0],
    viewerForward: [0, 0, 1],
    candidates: [
      {
        id: "trail-forage:0:0",
        subjectType: "object",
        name: "Trail Forage",
        role: "loot-cache",
        priority: 1,
        worldPosition: [0, 0, 1],
        prompts: ["use"],
      },
      {
        id: "kwama-brood:0:0",
        subjectType: "mob",
        name: "Kwama Brood",
        role: "mob-trail",
        priority: 8,
        worldPosition: [0, 0, 1.2],
        prompts: ["inspect"],
      },
    ],
  });

  expect(resolution.target?.id).toBe("kwama-brood:0:0");
  expect(resolution.target?.prompts[0]?.label).toBe("Inspect Kwama Brood");
});

test("interaction resolver surfaces visible cave mouths ahead of generic mob trails", () => {
  const resolution = resolveExplorationInteractionTarget({
    viewerPosition: [0, 0, 0],
    viewerForward: [0, 0, 1],
    candidates: [
      {
        id: "trail-forage:0:0",
        subjectType: "object",
        name: "Trail Forage",
        role: "loot-cache",
        priority: 1,
        worldPosition: [0, 0, 1],
        prompts: ["use"],
      },
      {
        id: "kwama-brood:0:0",
        subjectType: "mob",
        name: "Kwama Brood",
        role: "mob-trail",
        priority: 8,
        worldPosition: [0, 0, 1.2],
        prompts: ["inspect"],
      },
      {
        id: "cave-mouth:ash-ravine",
        subjectType: "zone",
        name: "Ash Ravine Mouth",
        role: "cave-mouth",
        priority: 12,
        worldPosition: [0, 0, 1.35],
        prompts: ["inspect"],
      },
    ],
  });

  expect(resolution.candidates.map((candidate) => candidate.role)).toEqual([
    "cave-mouth",
    "mob-trail",
    "loot-cache",
  ]);
  expect(resolution.target?.subjectType).toBe("zone");
  expect(resolution.target?.prompts[0]?.eventInput).toMatchObject({
    kind: "inspect",
    subjectType: "zone",
    role: "cave-mouth",
  });
});

test("interaction resolver keeps cave passage use payloads stable", () => {
  const resolution = resolveExplorationInteractionTarget({
    viewerPosition: [0, 0, 0],
    viewerForward: [0, 0, 1],
    candidates: [
      {
        id: "cave-exit:return",
        subjectType: "zone",
        name: "Cave Mouth Return",
        role: "cave-exit",
        priority: 18,
        worldPosition: [0, 0, 1],
        prompts: ["use"],
      },
      {
        id: "cave-passage:ash-kwama-ravines:ash-pilgrim-mine-mouth:ash-deep-brood-chamber",
        subjectType: "zone",
        name: "Ash Deep Brood Chamber Passage",
        role: "cave-passage",
        priority: 19,
        worldPosition: [0, 0, 1.2],
        prompts: [{ verb: "use", label: "Follow passage to Ash Deep Brood Chamber" }],
        payload: {
          caveTraversal: "passage",
          caveSystemId: "ash-kwama-ravines",
          fromCaveAnchorId: "ash-pilgrim-mine-mouth",
          toCaveAnchorId: "ash-deep-brood-chamber",
        },
      },
    ],
  });

  expect(resolution.target?.role).toBe("cave-passage");
  expect(resolution.target?.prompts[0]?.eventInput).toMatchObject({
    kind: "use",
    subjectType: "zone",
    subjectId: "cave-passage:ash-kwama-ravines:ash-pilgrim-mine-mouth:ash-deep-brood-chamber",
    role: "cave-passage",
    payload: {
      caveTraversal: "passage",
      caveSystemId: "ash-kwama-ravines",
      fromCaveAnchorId: "ash-pilgrim-mine-mouth",
      toCaveAnchorId: "ash-deep-brood-chamber",
    },
  });
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

test("interaction resolver carries repeatable occurrence ids for revisitable targets", () => {
  const resolution = resolveExplorationInteractionTarget({
    viewerPosition: [0, 0, 0],
    candidates: [{
      id: "berry-bush-forage:forage-patch:4:0",
      subjectType: "object",
      name: "Berry Bush Forage Patch",
      role: "loot-cache",
      worldPosition: [0, 0, 1],
      prompts: [{ verb: "use", label: "Revisit berry bush forage" }],
      occurrenceId: "revisit-2",
      repeatable: true,
      payload: {
        lootId: "berry-bush-forage",
        collectedBefore: true,
      },
    }],
  });

  expect(resolution.target?.prompts[0]?.eventInput).toMatchObject({
    kind: "use",
    subjectType: "object",
    subjectId: "berry-bush-forage:forage-patch:4:0",
    role: "loot-cache",
    occurrenceId: "revisit-2",
    repeatable: true,
    payload: {
      lootId: "berry-bush-forage",
      collectedBefore: true,
    },
  });
});
