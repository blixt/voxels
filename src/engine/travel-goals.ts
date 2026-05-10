export const TRAVEL_GOALS_VERSION = 1;

export type TravelGoalStepKind = "visit" | "inspect" | "read" | "use";
export type TravelGoalStatus = "inactive" | "active" | "completed";

export interface TravelGoalStepDefinition {
  id: string;
  kind: TravelGoalStepKind;
  targetId: string;
  label: string;
  optional?: boolean;
}

export interface TravelGoalDefinition {
  id: string;
  routeId: string;
  title: string;
  journalText: string;
  steps: readonly TravelGoalStepDefinition[];
}

export interface TravelGoalProgressInput {
  goalId?: string | null;
  routeId?: string | null;
  stepId?: string | null;
  kind: TravelGoalStepKind;
  targetId: string;
}

export interface TravelGoalSnapshot {
  id: string;
  routeId: string;
  title: string;
  journalText: string;
  status: TravelGoalStatus;
  completedStepIds: readonly string[];
  requiredStepCount: number;
  completedRequiredStepCount: number;
  progress: number;
  completed: boolean;
}

export interface RouteJournalSnapshot {
  goals: readonly TravelGoalSnapshot[];
  activeGoalIds: readonly string[];
  completedGoalIds: readonly string[];
}

export interface TravelGoalRecord {
  id: string;
  status: TravelGoalStatus;
  completedStepIds: readonly string[];
}

export interface RouteJournalState {
  version: typeof TRAVEL_GOALS_VERSION;
  goals: readonly TravelGoalRecord[];
}

export interface TravelGoalProgressResult {
  changed: boolean;
  completedGoalIds: readonly string[];
  completedStepIds: readonly string[];
  snapshot: RouteJournalSnapshot;
}

interface MutableTravelGoalRecord {
  id: string;
  status: TravelGoalStatus;
  completedStepIds: Set<string>;
}

export class RouteJournal {
  private readonly records = new Map<string, MutableTravelGoalRecord>();

  constructor(private readonly definitions: readonly TravelGoalDefinition[]) {}

  startGoal(goalId: string): TravelGoalProgressResult {
    const definition = this.findDefinition(goalId);
    if (!definition) {
      return this.result(false, [], []);
    }
    const record = this.ensureRecord(definition);
    if (record.status !== "inactive") {
      return this.result(false, [], []);
    }
    record.status = "active";
    return this.result(true, [], []);
  }

  observeProgress(input: TravelGoalProgressInput): TravelGoalProgressResult {
    const changedSteps: string[] = [];
    const completedGoals: string[] = [];
    let changed = false;

    for (const definition of this.matchDefinitions(input)) {
      const step = findMatchingStep(definition, input);
      if (!step) {
        continue;
      }
      const record = this.ensureRecord(definition);
      if (record.status === "inactive") {
        record.status = "active";
        changed = true;
      }
      if (record.completedStepIds.has(step.id)) {
        continue;
      }
      record.completedStepIds.add(step.id);
      changed = true;
      changedSteps.push(step.id);

      if (record.status !== "completed" && isGoalComplete(definition, record.completedStepIds)) {
        record.status = "completed";
        completedGoals.push(definition.id);
      }
    }

    return this.result(changed, completedGoals, changedSteps);
  }

  getSnapshot(): RouteJournalSnapshot {
    const goals = this.definitions.map((definition) => buildSnapshot(definition, this.records.get(definition.id)));
    return {
      goals,
      activeGoalIds: goals
        .filter((goal) => goal.status === "active")
        .map((goal) => goal.id),
      completedGoalIds: goals
        .filter((goal) => goal.status === "completed")
        .map((goal) => goal.id),
    };
  }

  exportState(): RouteJournalState {
    return {
      version: TRAVEL_GOALS_VERSION,
      goals: [...this.records.values()]
        .map((record) => ({
          id: record.id,
          status: record.status,
          completedStepIds: [...record.completedStepIds].sort((left, right) => left.localeCompare(right)),
        }))
        .sort((left, right) => left.id.localeCompare(right.id)),
    };
  }

  importState(state: Partial<RouteJournalState>): RouteJournalSnapshot {
    this.records.clear();
    for (const rawRecord of Array.isArray(state.goals) ? state.goals : []) {
      const record = normalizeImportedRecord(rawRecord, this.definitions);
      if (record) {
        this.records.set(record.id, record);
      }
    }
    return this.getSnapshot();
  }

  private matchDefinitions(input: TravelGoalProgressInput): readonly TravelGoalDefinition[] {
    if (input.goalId) {
      const definition = this.findDefinition(input.goalId);
      return definition ? [definition] : [];
    }
    return this.definitions.filter((definition) => !input.routeId || definition.routeId === input.routeId);
  }

  private findDefinition(goalId: string): TravelGoalDefinition | null {
    return this.definitions.find((definition) => definition.id === goalId) ?? null;
  }

  private ensureRecord(definition: TravelGoalDefinition): MutableTravelGoalRecord {
    const existing = this.records.get(definition.id);
    if (existing) {
      return existing;
    }
    const record: MutableTravelGoalRecord = {
      id: definition.id,
      status: "inactive",
      completedStepIds: new Set<string>(),
    };
    this.records.set(definition.id, record);
    return record;
  }

  private result(
    changed: boolean,
    completedGoalIds: readonly string[],
    completedStepIds: readonly string[],
  ): TravelGoalProgressResult {
    return {
      changed,
      completedGoalIds,
      completedStepIds,
      snapshot: this.getSnapshot(),
    };
  }
}

function findMatchingStep(
  definition: TravelGoalDefinition,
  input: TravelGoalProgressInput,
): TravelGoalStepDefinition | null {
  return definition.steps.find((step) =>
    (!input.stepId || step.id === input.stepId)
    && step.kind === input.kind
    && step.targetId === input.targetId
  ) ?? null;
}

function isGoalComplete(definition: TravelGoalDefinition, completedStepIds: ReadonlySet<string>): boolean {
  const requiredSteps = definition.steps.filter((step) => step.optional !== true);
  return requiredSteps.length > 0 && requiredSteps.every((step) => completedStepIds.has(step.id));
}

function buildSnapshot(
  definition: TravelGoalDefinition,
  record: MutableTravelGoalRecord | undefined,
): TravelGoalSnapshot {
  const completedStepIds = [...(record?.completedStepIds ?? new Set<string>())]
    .filter((stepId) => definition.steps.some((step) => step.id === stepId))
    .sort((left, right) => left.localeCompare(right));
  const requiredStepIds = definition.steps
    .filter((step) => step.optional !== true)
    .map((step) => step.id);
  const completedRequiredStepCount = requiredStepIds.filter((stepId) => completedStepIds.includes(stepId)).length;
  const requiredStepCount = requiredStepIds.length;
  const completed = requiredStepCount > 0 && completedRequiredStepCount >= requiredStepCount;
  return {
    id: definition.id,
    routeId: definition.routeId,
    title: definition.title,
    journalText: definition.journalText,
    status: completed ? "completed" : record?.status ?? "inactive",
    completedStepIds,
    requiredStepCount,
    completedRequiredStepCount,
    progress: requiredStepCount > 0 ? completedRequiredStepCount / requiredStepCount : 0,
    completed,
  };
}

function normalizeImportedRecord(
  value: unknown,
  definitions: readonly TravelGoalDefinition[],
): MutableTravelGoalRecord | null {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    return null;
  }
  const raw = value as Record<string, unknown>;
  const id = readNonEmptyString(raw.id);
  const definition = id ? definitions.find((candidate) => candidate.id === id) : null;
  if (!id || !definition || !isTravelGoalStatus(raw.status)) {
    return null;
  }
  const knownStepIds = new Set(definition.steps.map((step) => step.id));
  const completedStepIds = new Set(
    (Array.isArray(raw.completedStepIds) ? raw.completedStepIds : [])
      .filter((stepId): stepId is string => typeof stepId === "string" && knownStepIds.has(stepId)),
  );
  const status = isGoalComplete(definition, completedStepIds) ? "completed" : raw.status === "completed" ? "active" : raw.status;
  return {
    id,
    status,
    completedStepIds,
  };
}

function isTravelGoalStatus(value: unknown): value is TravelGoalStatus {
  return value === "inactive" || value === "active" || value === "completed";
}

function readNonEmptyString(value: unknown): string | null {
  return typeof value === "string" && value.trim().length > 0 ? value.trim() : null;
}
