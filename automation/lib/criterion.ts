import { access } from "node:fs/promises";
import path from "node:path";
import { rustTool } from "../../scripts/build-wasm.ts";
import { ScenarioArguments } from "./arguments.ts";
import { runProcess } from "./process.ts";
import type { ScenarioContext, ScenarioResult } from "./scenario.ts";

export interface CriterionBenchmark {
  readonly packageName: string;
  readonly benchName: string;
}

interface CriterionOptions {
  readonly filter?: string;
  readonly quick: boolean;
  readonly sampleSize?: number;
  readonly measurementSeconds?: number;
}

function parseOptions(values: readonly string[]): CriterionOptions {
  const arguments_ = new ScenarioArguments(values);
  const options = {
    filter: arguments_.string("filter"),
    quick: arguments_.flag("quick"),
    sampleSize: arguments_.number("sample-size", {
      integer: true,
      minimum: 10,
      maximum: 1_000_000,
    }),
    measurementSeconds: arguments_.number("measurement-seconds", {
      minimum: 0.05,
      maximum: 86_400,
    }),
  };
  if (options.quick && options.sampleSize !== undefined) {
    throw new Error("--quick and --sample-size cannot be combined");
  }
  arguments_.assertEmpty();
  return options;
}

export async function runCriterionBenchmark(
  context: ScenarioContext,
  values: readonly string[],
  benchmark: CriterionBenchmark,
): Promise<ScenarioResult> {
  const options = parseOptions(values);
  const outputDirectory = await context.artifacts.directoryFor("criterion");
  const criterionArguments = [
    ...(options.filter === undefined ? [] : [options.filter]),
    ...(options.quick ? ["--quick"] : []),
    ...(options.sampleSize === undefined ? [] : ["--sample-size", String(options.sampleSize)]),
    ...(options.measurementSeconds === undefined
      ? []
      : ["--measurement-time", String(options.measurementSeconds)]),
  ];
  await runProcess(
    context,
    rustTool("cargo"),
    [
      "bench",
      "-p",
      benchmark.packageName,
      "--bench",
      benchmark.benchName,
      ...(criterionArguments.length === 0 ? [] : ["--", ...criterionArguments]),
    ],
    {
      label: `${benchmark.packageName} Criterion benchmark`,
      stdio: "inherit",
      env: { ...process.env, CRITERION_HOME: outputDirectory },
    },
  );

  const report = path.join(outputDirectory, "report", "index.html");
  try {
    await access(report);
    context.artifacts.record("Criterion report", report, "text/html");
  } catch {
    throw new Error(`${benchmark.packageName} Criterion run did not produce its HTML report`);
  }
  await context.artifacts.writeJson("Criterion run", "criterion-run.json", {
    schemaVersion: 1,
    benchmark,
    options,
  });
  return {
    summary: `Completed ${benchmark.packageName}/${benchmark.benchName} Criterion benchmark.`,
    metrics: {
      package: benchmark.packageName,
      bench: benchmark.benchName,
      ...(options.filter === undefined ? {} : { filter: options.filter }),
    },
  };
}
