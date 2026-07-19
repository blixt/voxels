import { test } from "vite-plus/test";
import { runScenario, type ScenarioDefinition, type ScenarioManifest } from "./scenario.ts";

export interface ScenarioTestOptions {
  readonly arguments?: readonly string[];
  readonly timeoutMs?: number;
  readonly artifactsRoot?: string;
  readonly validate?: (manifest: ScenarioManifest) => void | Promise<void>;
}

/**
 * Registers any scenario as a Vite+ test. The scenario still owns setup, screenshots, metrics,
 * assertions, and cleanup; this adapter only connects its pass/fail lifecycle to the test runner.
 */
export function scenarioTest(
  definition: ScenarioDefinition,
  options: ScenarioTestOptions = {},
): void {
  test(
    definition.summary,
    async () => {
      const manifest = await runScenario(definition, options.arguments ?? [], {
        artifacts: {
          root: options.artifactsRoot ?? "target/automation-tests",
        },
        log: () => {},
      });
      await options.validate?.(manifest);
    },
    options.timeoutMs ?? definition.timeoutMs,
  );
}
