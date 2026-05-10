import type {
  ExplorationEventInput,
  ExplorationEventKind,
  ExplorationEventSubjectType,
} from "./exploration-events.ts";

export type ExplorationInteractionVerb = Extract<ExplorationEventKind, "inspect" | "read" | "use">;

export interface ExplorationInteractionPrompt {
  verb: ExplorationInteractionVerb;
  label?: string;
  description?: string | null;
  disabled?: boolean;
}

export interface ExplorationInteractionCandidate {
  id: string;
  subjectType: ExplorationEventSubjectType;
  name?: string;
  role?: string | null;
  worldPosition: readonly [number, number, number];
  interactionRadiusMeters?: number;
  priority?: number;
  prompts: readonly (ExplorationInteractionVerb | ExplorationInteractionPrompt)[];
  flavorText?: string | null;
  skillAwards?: ExplorationEventInput["skillAwards"];
  payload?: ExplorationEventInput["payload"];
}

export interface ExplorationInteractionResolverInput {
  viewerPosition: readonly [number, number, number];
  viewerForward?: readonly [number, number, number] | null;
  maxDistanceMeters?: number;
  candidates: readonly ExplorationInteractionCandidate[];
}

export interface ResolvedExplorationInteractionPrompt {
  verb: ExplorationInteractionVerb;
  label: string;
  description: string | null;
  disabled: boolean;
  eventInput: ExplorationEventInput;
}

export interface ResolvedExplorationInteractionTarget {
  id: string;
  subjectType: ExplorationEventSubjectType;
  name: string;
  role: string;
  worldPosition: readonly [number, number, number];
  distanceMeters: number;
  facingAlignment: number | null;
  prompts: readonly ResolvedExplorationInteractionPrompt[];
}

export interface ExplorationInteractionResolution {
  target: ResolvedExplorationInteractionTarget | null;
  candidates: readonly ResolvedExplorationInteractionTarget[];
}

const DEFAULT_INTERACTION_RADIUS_METERS = 4;
const DEFAULT_MAX_DISTANCE_METERS = 6;
const VERB_ORDER = new Map<ExplorationInteractionVerb, number>([
  ["inspect", 0],
  ["read", 1],
  ["use", 2],
]);

export function resolveExplorationInteractionTarget(
  input: ExplorationInteractionResolverInput,
): ExplorationInteractionResolution {
  const maxDistanceMeters = readPositiveNumber(input.maxDistanceMeters) ?? DEFAULT_MAX_DISTANCE_METERS;
  const viewerForward = normalizeVector(input.viewerForward);
  const resolved: ResolvedExplorationInteractionTarget[] = [];

  for (const candidate of input.candidates) {
    const normalized = normalizeCandidate(candidate, input.viewerPosition, viewerForward, maxDistanceMeters);
    if (normalized) {
      resolved.push(normalized);
    }
  }

  resolved.sort((left, right) => compareInteractionTargets(left, right, input.candidates));
  return {
    target: resolved[0] ?? null,
    candidates: resolved,
  };
}

export function buildExplorationInteractionEventInput(
  target: Pick<ResolvedExplorationInteractionTarget, "id" | "subjectType" | "name" | "role" | "worldPosition">,
  verb: ExplorationInteractionVerb,
  options: Pick<ExplorationEventInput, "flavorText" | "payload" | "repeatable"> = {},
): ExplorationEventInput {
  return {
    kind: verb,
    subjectType: target.subjectType,
    subjectId: target.id,
    role: target.role,
    name: target.name,
    worldPosition: target.worldPosition,
    ...options,
  };
}

function normalizeCandidate(
  candidate: ExplorationInteractionCandidate,
  viewerPosition: readonly [number, number, number],
  viewerForward: readonly [number, number, number] | null,
  maxDistanceMeters: number,
): ResolvedExplorationInteractionTarget | null {
  const id = readNonEmptyString(candidate.id);
  if (!id) {
    return null;
  }
  const name = readNonEmptyString(candidate.name) ?? id;
  const role = readNonEmptyString(candidate.role) ?? "default";
  const interactionRadiusMeters = readPositiveNumber(candidate.interactionRadiusMeters) ?? DEFAULT_INTERACTION_RADIUS_METERS;
  const distanceMeters = distanceBetween(viewerPosition, candidate.worldPosition);
  if (distanceMeters > Math.min(maxDistanceMeters, interactionRadiusMeters)) {
    return null;
  }

  const prompts = normalizePrompts(candidate, { id, name, role });
  if (prompts.length === 0) {
    return null;
  }

  return {
    id,
    subjectType: candidate.subjectType,
    name,
    role,
    worldPosition: [...candidate.worldPosition],
    distanceMeters,
    facingAlignment: resolveFacingAlignment(viewerPosition, candidate.worldPosition, viewerForward),
    prompts,
  };
}

function normalizePrompts(
  candidate: ExplorationInteractionCandidate,
  target: Pick<ResolvedExplorationInteractionTarget, "id" | "name" | "role">,
): ResolvedExplorationInteractionPrompt[] {
  const prompts = new Map<ExplorationInteractionVerb, ResolvedExplorationInteractionPrompt>();
  for (const rawPrompt of candidate.prompts) {
    const prompt = normalizePrompt(rawPrompt);
    if (!prompt || prompts.has(prompt.verb)) {
      continue;
    }
    prompts.set(prompt.verb, {
      verb: prompt.verb,
      label: readNonEmptyString(prompt.label) ?? buildDefaultPromptLabel(prompt.verb, target.name),
      description: typeof prompt.description === "string" ? prompt.description : null,
      disabled: prompt.disabled === true,
      eventInput: {
        kind: prompt.verb,
        subjectType: candidate.subjectType,
        subjectId: target.id,
        role: target.role,
        name: target.name,
        flavorText: typeof candidate.flavorText === "string" ? candidate.flavorText : null,
        worldPosition: candidate.worldPosition,
        ...(candidate.skillAwards !== undefined ? { skillAwards: candidate.skillAwards } : {}),
        ...(candidate.payload !== undefined ? { payload: candidate.payload } : {}),
      },
    });
  }
  return [...prompts.values()].sort((left, right) => VERB_ORDER.get(left.verb)! - VERB_ORDER.get(right.verb)!);
}

function normalizePrompt(
  value: ExplorationInteractionVerb | ExplorationInteractionPrompt,
): ExplorationInteractionPrompt | null {
  if (typeof value === "string") {
    return isInteractionVerb(value) ? { verb: value } : null;
  }
  return isInteractionVerb(value.verb) ? value : null;
}

function compareInteractionTargets(
  left: ResolvedExplorationInteractionTarget,
  right: ResolvedExplorationInteractionTarget,
  sourceCandidates: readonly ExplorationInteractionCandidate[],
): number {
  const leftPriority = readPriority(sourceCandidates, left.id);
  const rightPriority = readPriority(sourceCandidates, right.id);
  if (leftPriority !== rightPriority) {
    return rightPriority - leftPriority;
  }
  const leftFacing = left.facingAlignment ?? 0;
  const rightFacing = right.facingAlignment ?? 0;
  if (leftFacing !== rightFacing) {
    return rightFacing - leftFacing;
  }
  if (left.distanceMeters !== right.distanceMeters) {
    return left.distanceMeters - right.distanceMeters;
  }
  return left.id.localeCompare(right.id);
}

function readPriority(candidates: readonly ExplorationInteractionCandidate[], id: string): number {
  return candidates.find((candidate) => candidate.id === id)?.priority ?? 0;
}

function resolveFacingAlignment(
  from: readonly [number, number, number],
  to: readonly [number, number, number],
  viewerForward: readonly [number, number, number] | null,
): number | null {
  if (!viewerForward) {
    return null;
  }
  const toTarget = normalizeVector([to[0] - from[0], to[1] - from[1], to[2] - from[2]]);
  if (!toTarget) {
    return 1;
  }
  return toTarget[0] * viewerForward[0] + toTarget[1] * viewerForward[1] + toTarget[2] * viewerForward[2];
}

function normalizeVector(value: readonly [number, number, number] | null | undefined): readonly [number, number, number] | null {
  if (!value || !value.every(Number.isFinite)) {
    return null;
  }
  const length = Math.hypot(value[0], value[1], value[2]);
  if (length <= 0) {
    return null;
  }
  return [value[0] / length, value[1] / length, value[2] / length];
}

function distanceBetween(left: readonly [number, number, number], right: readonly [number, number, number]): number {
  return Math.hypot(left[0] - right[0], left[1] - right[1], left[2] - right[2]);
}

function isInteractionVerb(value: string): value is ExplorationInteractionVerb {
  return value === "inspect" || value === "read" || value === "use";
}

function buildDefaultPromptLabel(verb: ExplorationInteractionVerb, name: string): string {
  switch (verb) {
    case "inspect":
      return `Inspect ${name}`;
    case "read":
      return `Read ${name}`;
    case "use":
      return `Use ${name}`;
  }
}

function readNonEmptyString(value: unknown): string | null {
  return typeof value === "string" && value.trim().length > 0 ? value.trim() : null;
}

function readPositiveNumber(value: unknown): number | null {
  return typeof value === "number" && Number.isFinite(value) && value > 0 ? value : null;
}
