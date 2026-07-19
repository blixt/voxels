import type { ScenarioDefinition } from "../lib/scenario.ts";
import benchCore from "./bench-core.ts";
import benchRuntime from "./bench-runtime.ts";
import benchWorld from "./bench-world.ts";
import botLoad from "./bot-load.ts";
import networkCompare from "./network-compare.ts";
import networkBenchmark from "./network-benchmark.ts";
import multiplayer from "./multiplayer.ts";
import renderProfile from "./render-profile.ts";
import terrainLightingCompare from "./terrain-lighting-compare.ts";
import weatherMotion from "./weather-motion.ts";
import worldLab from "./world-lab.ts";

const definitions = [
  benchCore,
  benchRuntime,
  benchWorld,
  botLoad,
  networkCompare,
  networkBenchmark,
  multiplayer,
  renderProfile,
  terrainLightingCompare,
  weatherMotion,
  worldLab,
] as const;

export const scenarios: readonly ScenarioDefinition[] = Object.freeze(
  definitions.toSorted((left, right) => left.id.localeCompare(right.id)),
);

export function scenarioById(id: string): ScenarioDefinition | undefined {
  return scenarios.find((scenario) => scenario.id === id);
}
