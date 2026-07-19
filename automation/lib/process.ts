import { spawn, type ChildProcess, type SpawnOptions } from "node:child_process";
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

export function setScenarioEnvironment(
  context: ScenarioContext,
  name: string,
  value: string,
): void {
  const previous = process.env[name];
  process.env[name] = value;
  context.defer(`environment ${name}`, () => {
    if (previous === undefined) delete process.env[name];
    else process.env[name] = previous;
  });
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

function signalProcessTree(child: ChildProcess, signal: NodeJS.Signals): void {
  if (child.exitCode !== null || child.signalCode !== null) return;
  try {
    if (process.platform !== "win32" && child.pid !== undefined) process.kill(-child.pid, signal);
    else child.kill(signal);
  } catch (error) {
    if (!(error instanceof Error && "code" in error && error.code === "ESRCH")) throw error;
  }
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
  let stopped = false;
  const stop = async (signal = stopSignal): Promise<void> => {
    if (stopped || child.exitCode !== null || child.signalCode !== null) return;
    stopped = true;
    signalProcessTree(child, signal);
    try {
      await Promise.race([completed, new Promise<void>((resolve) => setTimeout(resolve, 2_000))]);
    } catch (error) {
      if (child.signalCode !== signal) throw error;
    }
    if (child.exitCode === null && child.signalCode === null) {
      signalProcessTree(child, "SIGKILL");
      try {
        await completed;
      } catch (error) {
        if (child.signalCode !== "SIGKILL") throw error;
      }
    }
  };
  const abort = (): void => {
    void stop();
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
}
