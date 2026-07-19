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

export function startProcess(
  context: ScenarioContext,
  command: string,
  arguments_: readonly string[],
  options: StartProcessOptions,
): ManagedProcess {
  context.throwIfAborted();
  const child = spawn(command, [...arguments_], {
    ...options,
    signal: context.signal,
  });
  const completed = completion(child, options.label);
  let stopped = false;
  const stop = async (signal = options.stopSignal ?? "SIGTERM"): Promise<void> => {
    if (stopped || child.exitCode !== null || child.signalCode !== null) return;
    stopped = true;
    child.kill(signal);
    try {
      await completed;
    } catch (error) {
      if (child.signalCode !== signal) throw error;
    }
  };
  context.defer(`process ${options.label}`, stop);
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
