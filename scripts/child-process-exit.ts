import { spawn, type ChildProcess, type SpawnOptions } from "node:child_process";
import { terminateProcessTree } from "./process-tree.ts";

interface SignalProcess {
  readonly pid: number;
  exitCode: string | number | null | undefined;
  on(signal: NodeJS.Signals, listener: () => void): unknown;
  removeListener(signal: NodeJS.Signals, listener: () => void): unknown;
  kill(pid: number, signal: NodeJS.Signals): true;
}

interface OwnedChildExitOptions {
  readonly processTarget?: SignalProcess;
  readonly terminationTimeoutMs?: number;
  readonly terminate?: typeof terminateProcessTree;
}

function reportSpawnError(processTarget: SignalProcess, error: unknown): void {
  console.error(error);
  processTarget.exitCode = 1;
}

function mirrorExit(
  processTarget: SignalProcess,
  code: number | null,
  signal: NodeJS.Signals | null,
): void {
  if (code !== null) {
    processTarget.exitCode = code;
  } else if (signal !== null) {
    processTarget.kill(processTarget.pid, signal);
  } else {
    processTarget.exitCode = 1;
  }
}

export function spawnOwnedChild(
  command: string,
  arguments_: readonly string[],
  options: SpawnOptions,
): ChildProcess {
  return spawn(command, [...arguments_], {
    ...options,
    detached: options.detached ?? process.platform !== "win32",
  });
}

/** Mirrors child exit while forwarding wrapper termination to the child's complete process tree. */
export function mirrorOwnedChildExit(
  child: ChildProcess,
  {
    processTarget = process,
    terminationTimeoutMs = 2_000,
    terminate = terminateProcessTree,
  }: OwnedChildExitOptions = {},
): void {
  const signals: NodeJS.Signals[] = ["SIGINT", "SIGTERM"];
  const handlers = new Map<NodeJS.Signals, () => void>();
  let spawnFailed = false;
  let termination: Promise<void> | undefined;
  let terminationFailed = false;

  const detach = (): void => {
    for (const [signal, handler] of handlers) processTarget.removeListener(signal, handler);
    handlers.clear();
  };
  for (const signal of signals) {
    const handler = (): void => {
      if (termination !== undefined) return;
      termination = terminate(child, terminationTimeoutMs, false, signal);
      void termination.catch((error) => {
        terminationFailed = true;
        reportSpawnError(processTarget, error);
      });
    };
    handlers.set(signal, handler);
    processTarget.on(signal, handler);
  }

  child.on("error", (error) => {
    spawnFailed = true;
    detach();
    reportSpawnError(processTarget, error);
  });
  child.on("close", (code, signal) => {
    void (async () => {
      if (termination !== undefined) await termination.catch(() => {});
      detach();
      if (!spawnFailed && !terminationFailed) mirrorExit(processTarget, code, signal);
    })();
  });
}
