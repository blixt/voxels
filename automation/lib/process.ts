import { spawn, type ChildProcess, type SpawnOptions } from "node:child_process";
import { terminateProcessTree } from "../../scripts/process-tree.ts";
import type { ScenarioContext } from "./scenario.ts";

export interface ManagedProcess {
  readonly child: ChildProcess;
  readonly completed: Promise<void>;
  stop(signal?: NodeJS.Signals): Promise<void>;
}

export interface StartProcessOptions extends Omit<SpawnOptions, "signal"> {
  readonly label: string;
  readonly stopSignal?: NodeJS.Signals;
}

function completion(child: ChildProcess, label: string): Promise<void> {
  return new Promise((resolve, reject) => {
    child.once("error", reject);
    child.once("exit", (code, signal) => {
      if (code === 0) resolve();
      else {
        reject(
          new Error(
            `${label} exited with ${signal === null ? `status ${code ?? "unknown"}` : `signal ${signal}`}`,
          ),
        );
      }
    });
  });
}

export function startProcess(
  context: ScenarioContext,
  command: string,
  arguments_: readonly string[],
  options: StartProcessOptions,
): ManagedProcess {
  context.throwIfAborted();
  const { label, stopSignal = "SIGTERM", ...spawnOptions } = options;
  const child = spawn(command, [...arguments_], {
    ...spawnOptions,
    detached: spawnOptions.detached ?? process.platform !== "win32",
  });
  const completed = completion(child, label);
  let stopPromise: Promise<void> | undefined;
  const stop = (signal = stopSignal): Promise<void> => {
    if (stopPromise !== undefined) return stopPromise;
    stopPromise = (async () => {
      await terminateProcessTree(child, 2_000, false, signal);
      try {
        await completed;
      } catch (error) {
        if (child.signalCode !== signal && child.signalCode !== "SIGKILL") throw error;
      }
    })();
    return stopPromise;
  };
  const abort = (): void => {
    // Cleanup awaits this same promise and reports any termination failure. Attach a handler here
    // so a failure cannot become an unhandled rejection before cleanup starts.
    void stop().catch(() => {});
  };
  context.signal.addEventListener("abort", abort, { once: true });
  void completed.finally(() => context.signal.removeEventListener("abort", abort)).catch(() => {});
  context.defer(`process ${label}`, stop);
  return Object.freeze({ child, completed, stop });
}

export async function runProcess(
  context: ScenarioContext,
  command: string,
  arguments_: readonly string[],
  options: StartProcessOptions,
): Promise<void> {
  await startProcess(context, command, arguments_, options).completed;
  context.throwIfAborted();
}
