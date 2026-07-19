import type { ScenarioDefinition } from "../lib/scenario.ts";
import benchCore from "./bench-core.ts";
import benchRuntime from "./bench-runtime.ts";
import benchWorld from "./bench-world.ts";
import worldLab from "./world-lab.ts";

const definitions = [benchCore, benchRuntime, benchWorld, worldLab] as const;

export const scenarios: readonly ScenarioDefinition[] = Object.freeze(
  definitions.toSorted((left, right) => left.id.localeCompare(right.id)),
);

export function scenarioById(id: string): ScenarioDefinition | undefined {
  return scenarios.find((scenario) => scenario.id === id);
}
