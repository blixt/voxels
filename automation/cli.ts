import { inspect } from "node:util";
import type { ScenarioDefinition } from "./lib/scenario.ts";
import { runScenario } from "./lib/scenario.ts";
import { loadScenarios, scenarioById } from "./scenarios/registry.ts";

function usage(): never {
  throw new Error(
    "usage: vp run automation -- list | describe <scenario> | run <scenario> [scenario options]",
  );
}

function printScenario(scenario: ScenarioDefinition): void {
  console.log(
    JSON.stringify(
      {
        id: scenario.id,
        kind: scenario.kind,
        summary: scenario.summary,
        uses: scenario.uses,
        timeoutMs: scenario.timeoutMs ?? null,
      },
      null,
      2,
    ),
  );
}

async function main(arguments_: readonly string[]): Promise<void> {
  const [command = "list", id, ...scenarioArguments] = arguments_.filter(
    (argument) => argument !== "--",
  );
  if (command === "list") {
    const scenarios = await loadScenarios();
    for (const scenario of scenarios) {
      console.log(
        `${scenario.id.padEnd(24)} ${scenario.kind.padEnd(10)} ${scenario.summary} [${Object.entries(
          scenario.uses,
        )
          .filter(([, enabled]) => enabled !== false && enabled !== undefined)
          .map(([name, value]) => (value === true ? name : `${name}:${value}`))
          .join(", ")}]`,
      );
    }
    return;
  }
  if (id === undefined) usage();
  const scenario = await scenarioById(id);
  if (scenario === undefined) {
    throw new Error(`unknown automation scenario ${id}; run 'vp run automation -- list'`);
  }
  if (command === "describe") {
    if (scenarioArguments.length > 0) usage();
    printScenario(scenario);
    return;
  }
  if (command !== "run") usage();
  const manifest = await runScenario(scenario, scenarioArguments, {
    installSignalHandlers: true,
  });
  console.log(
    JSON.stringify(
      {
        status: manifest.status,
        scenario: manifest.scenario.id,
        durationMs: manifest.durationMs,
        artifacts: manifest.artifacts,
        result: manifest.result ?? null,
      },
      null,
      2,
    ),
  );
}

try {
  await main(process.argv.slice(2));
} catch (error) {
  console.error(error instanceof Error && error.stack ? error.stack : inspect(error));
  process.exitCode = 1;
}
