import { readdir } from "node:fs/promises";
import { dirname, extname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import type { ScenarioDefinition } from "../lib/scenario.ts";

const directory = dirname(fileURLToPath(import.meta.url));
let cached: Promise<readonly ScenarioDefinition[]> | undefined;

function isScenarioDefinition(value: unknown): value is ScenarioDefinition {
  if (typeof value !== "object" || value === null) return false;
  const candidate = value as Partial<ScenarioDefinition>;
  return (
    typeof candidate.id === "string" &&
    typeof candidate.kind === "string" &&
    typeof candidate.summary === "string" &&
    typeof candidate.uses === "object" &&
    candidate.uses !== null &&
    typeof candidate.run === "function"
  );
}

async function discoverScenarios(): Promise<readonly ScenarioDefinition[]> {
  const entries = await readdir(directory, { withFileTypes: true });
  const modules = entries
    .filter(
      (entry) =>
        entry.isFile() &&
        entry.name !== "registry.ts" &&
        !entry.name.endsWith(".test.ts") &&
        extname(entry.name) === ".ts",
    )
    .map(async (entry) => {
      const module: unknown = await import(pathToFileURL(join(directory, entry.name)).href);
      const definition =
        typeof module === "object" && module !== null && "default" in module
          ? module.default
          : undefined;
      if (!isScenarioDefinition(definition)) {
        throw new Error(`automation/scenarios/${entry.name} has no scenario default export`);
      }
      return definition;
    });
  const definitions = (await Promise.all(modules)).toSorted((left, right) =>
    left.id.localeCompare(right.id),
  );
  const ids = new Set<string>();
  for (const definition of definitions) {
    if (ids.has(definition.id)) throw new Error(`duplicate automation scenario ${definition.id}`);
    ids.add(definition.id);
  }
  return Object.freeze(definitions);
}

export function loadScenarios(): Promise<readonly ScenarioDefinition[]> {
  cached ??= discoverScenarios();
  return cached;
}

export async function scenarioById(id: string): Promise<ScenarioDefinition | undefined> {
  return (await loadScenarios()).find((scenario) => scenario.id === id);
}
