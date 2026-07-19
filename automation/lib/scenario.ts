import { inspect } from "node:util";
import { ArtifactStore, type ArtifactOptions, type ArtifactRecord } from "./artifacts.ts";

export type ScenarioKind = "validation" | "benchmark" | "capture" | "bot-load" | "analysis";
export type ViewportKind = "browser" | "native";

export interface ScenarioUses {
  readonly world?: boolean;
  readonly viewport?: ViewportKind;
  readonly screenshots?: boolean;
  readonly video?: boolean;
  readonly trace?: boolean;
  readonly bots?: boolean;
  readonly network?: boolean;
  readonly metrics?: boolean;
  readonly rust?: boolean;
}

export interface ScenarioResult {
  readonly summary?: string;
  readonly metrics?: Readonly<Record<string, unknown>>;
  readonly details?: unknown;
}

export interface ScenarioDefinition {
  readonly id: string;
  readonly kind: ScenarioKind;
  readonly summary: string;
  readonly uses: Readonly<ScenarioUses>;
  readonly timeoutMs?: number;
  run(context: ScenarioContext, arguments_: readonly string[]): Promise<ScenarioResult | void>;
}

export interface ScenarioManifest {
  readonly schemaVersion: 1;
  readonly scenario: {
    readonly id: string;
    readonly kind: ScenarioKind;
    readonly summary: string;
    readonly uses: Readonly<ScenarioUses>;
  };
  readonly commandArguments: readonly string[];
  readonly startedAt: string;
  readonly finishedAt: string;
  readonly durationMs: number;
  readonly status: "passed" | "failed";
  readonly result?: ScenarioResult;
  readonly error?: string;
  readonly artifacts: readonly ArtifactRecord[];
}

export interface ScenarioRunOptions {
  readonly artifacts?: ArtifactOptions;
  readonly installSignalHandlers?: boolean;
  readonly log?: (message: string) => void;
}

type Cleanup = () => void | Promise<void>;

export class ScenarioContext {
  readonly artifacts: ArtifactStore;
  readonly signal: AbortSignal;
  readonly definition: ScenarioDefinition;

  readonly #cleanups: { readonly label: string; readonly cleanup: Cleanup }[] = [];
  readonly #log: (message: string) => void;

  constructor(
    definition: ScenarioDefinition,
    artifacts: ArtifactStore,
    signal: AbortSignal,
    log: (message: string) => void,
  ) {
    this.definition = definition;
    this.artifacts = artifacts;
    this.signal = signal;
    this.#log = log;
  }

  log(message: string): void {
    this.#log(`[${this.definition.id}] ${message}`);
  }

  defer(label: string, cleanup: Cleanup): void {
    this.#cleanups.push({ label, cleanup });
  }

  throwIfAborted(): void {
    if (this.signal.aborted) {
      throw this.signal.reason instanceof Error
        ? this.signal.reason
        : new Error("automation scenario aborted");
    }
  }

  async cleanup(): Promise<Error[]> {
    const errors: Error[] = [];
    for (const entry of this.#cleanups.toReversed()) {
      try {
        await entry.cleanup();
      } catch (error) {
        errors.push(
          new Error(
            `cleanup ${entry.label} failed: ${error instanceof Error ? error.message : String(error)}`,
            { cause: error },
          ),
        );
      }
    }
    this.#cleanups.length = 0;
    return errors;
  }
}

function validateDefinition(definition: ScenarioDefinition): void {
  if (!/^[a-z0-9]+(?:-[a-z0-9]+)*$/u.test(definition.id)) {
    throw new Error(`scenario id must be kebab-case: ${definition.id}`);
  }
  if (definition.summary.trim().length === 0) {
    throw new Error(`scenario ${definition.id} must have a summary`);
  }
  const uses = definition.uses;
  if ((uses.screenshots || uses.video || uses.trace) && uses.viewport === undefined) {
    throw new Error(`scenario ${definition.id} captures a viewport without declaring one`);
  }
  if (uses.viewport === "native") {
    throw new Error(
      `scenario ${definition.id} requests the unavailable native viewport; add renderer parity first`,
    );
  }
  if (uses.bots && !uses.world) {
    throw new Error(`scenario ${definition.id} declares bots without a world service`);
  }
}

export function defineScenario<const Definition extends ScenarioDefinition>(
  definition: Definition,
): Definition {
  validateDefinition(definition);
  return Object.freeze({
    ...definition,
    uses: Object.freeze({ ...definition.uses }),
  });
}

function errorText(error: unknown): string {
  return error instanceof Error && error.stack ? error.stack : inspect(error);
}

export async function runScenario(
  definition: ScenarioDefinition,
  arguments_: readonly string[],
  options: ScenarioRunOptions = {},
): Promise<ScenarioManifest> {
  validateDefinition(definition);
  const artifacts = await ArtifactStore.create(definition.id, options.artifacts);
  const abort = new AbortController();
  const log = options.log ?? console.log;
  const context = new ScenarioContext(definition, artifacts, abort.signal, log);
  const started = new Date();
  const timeout =
    definition.timeoutMs === undefined
      ? undefined
      : setTimeout(
          () => abort.abort(new Error(`scenario timed out after ${definition.timeoutMs}ms`)),
          definition.timeoutMs,
        );
  const signals: NodeJS.Signals[] = ["SIGINT", "SIGTERM"];
  const onSignal = (signal: NodeJS.Signals): void => {
    abort.abort(new Error(`scenario received ${signal}`));
  };
  if (options.installSignalHandlers) {
    for (const signal of signals) process.once(signal, onSignal);
  }

  let result: ScenarioResult | undefined;
  let failure: unknown;
  try {
    context.throwIfAborted();
    result = (await definition.run(context, arguments_)) ?? {};
    context.throwIfAborted();
  } catch (error) {
    failure = error;
  } finally {
    if (timeout !== undefined) clearTimeout(timeout);
    if (options.installSignalHandlers) {
      for (const signal of signals) process.off(signal, onSignal);
    }
    const cleanupErrors = await context.cleanup();
    if (cleanupErrors.length > 0) {
      failure =
        failure === undefined
          ? new AggregateError(cleanupErrors, "scenario cleanup failed")
          : new AggregateError([failure, ...cleanupErrors], "scenario and cleanup failed");
    }
  }

  const finished = new Date();
  const manifest: ScenarioManifest = {
    schemaVersion: 1,
    scenario: {
      id: definition.id,
      kind: definition.kind,
      summary: definition.summary,
      uses: definition.uses,
    },
    commandArguments: [...arguments_],
    startedAt: started.toISOString(),
    finishedAt: finished.toISOString(),
    durationMs: finished.getTime() - started.getTime(),
    status: failure === undefined ? "passed" : "failed",
    ...(result === undefined ? {} : { result }),
    ...(failure === undefined ? {} : { error: errorText(failure) }),
    artifacts: artifacts.records,
  };
  await artifacts.writeJson("scenario manifest", "manifest.json", manifest);
  if (failure !== undefined) throw failure;
  return manifest;
}
