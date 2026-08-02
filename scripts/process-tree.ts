import { type ChildProcess, execFileSync } from "node:child_process";

function wait(milliseconds: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, milliseconds));
}

function signalProcessIds(processIds: Iterable<number>, signal: NodeJS.Signals): void {
  for (const pid of processIds) {
    try {
      process.kill(pid, signal);
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code !== "ESRCH") throw error;
    }
  }
}

export function signalOwnedProcessGroup(
  rootPid: number,
  signal: NodeJS.Signals,
  kill: (pid: number, signal: NodeJS.Signals) => true = (pid, signal) => process.kill(pid, signal),
): "signaled" | "missing" | "forbidden" {
  try {
    kill(-rootPid, signal);
    return "signaled";
  } catch (error) {
    const code = (error as NodeJS.ErrnoException).code;
    if (code === "ESRCH") return "missing";
    if (code === "EPERM") return "forbidden";
    throw error;
  }
}

function ownedProcessTreeIds(rootPid: number): number[] {
  let rows: string;
  try {
    rows = execFileSync("/bin/ps", ["-ax", "-o", "pid=,ppid=,pgid="], {
      encoding: "utf8",
    });
  } catch {
    return [];
  }
  const childrenByParent = new Map<number, number[]>();
  const processGroupMembers = new Set<number>();
  for (const row of rows.split("\n")) {
    const match = /^\s*(\d+)\s+(\d+)\s+(\d+)\s*$/u.exec(row);
    if (!match) continue;
    const pid = Number(match[1]);
    const parentPid = Number(match[2]);
    const processGroup = Number(match[3]);
    if (processGroup === rootPid && pid !== rootPid) processGroupMembers.add(pid);
    const children = childrenByParent.get(parentPid);
    if (children) children.push(pid);
    else childrenByParent.set(parentPid, [pid]);
  }
  const descendants: number[] = [];
  const pending = [...(childrenByParent.get(rootPid) ?? [])];
  while (pending.length > 0) {
    const pid = pending.pop();
    if (pid === undefined) break;
    descendants.push(pid);
    pending.push(...(childrenByParent.get(pid) ?? []));
  }
  return [...new Set([...descendants, ...processGroupMembers])];
}

function signalProcessTree(
  child: ChildProcess,
  signal: NodeJS.Signals,
  forceDirectFallback: boolean,
): number[] {
  if (child.exitCode !== null || child.signalCode !== null) return [];
  const descendants =
    process.platform === "win32" || child.pid === undefined ? [] : ownedProcessTreeIds(child.pid);
  try {
    if (
      !forceDirectFallback &&
      process.platform !== "win32" &&
      child.pid !== undefined &&
      signalOwnedProcessGroup(child.pid, signal) === "signaled"
    ) {
      return descendants;
    }
    child.kill(signal);
    signalProcessIds(descendants.reverse(), signal);
    return descendants;
  } catch (error) {
    const code = (error as NodeJS.ErrnoException).code;
    if (code === "ESRCH") return descendants;
    throw error;
  }
}

function processExists(pid: number): boolean {
  try {
    process.kill(pid, 0);
    return true;
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === "ESRCH") return false;
    throw error;
  }
}

async function waitForProcessIdsExit(
  processIds: Iterable<number>,
  timeoutMs: number,
): Promise<number[]> {
  let survivors = [...new Set(processIds)].filter(processExists);
  const deadline = Date.now() + timeoutMs;
  while (survivors.length > 0 && Date.now() < deadline) {
    await wait(Math.min(25, Math.max(1, deadline - Date.now())));
    survivors = survivors.filter(processExists);
  }
  return survivors;
}

async function forceKillOwnedProcessIds(
  rootPid: number | undefined,
  processIds: Iterable<number>,
  timeoutMs: number,
): Promise<number[]> {
  const deadline = Date.now() + timeoutMs;
  let survivors = new Set(processIds);
  while (true) {
    if (process.platform !== "win32" && rootPid !== undefined) {
      for (const pid of ownedProcessTreeIds(rootPid)) survivors.add(pid);
    }
    const live = [...survivors].filter(processExists);
    if (live.length === 0) return [];
    signalProcessIds(live, "SIGKILL");
    if (Date.now() >= deadline) return live.filter(processExists);
    await wait(Math.min(25, Math.max(1, deadline - Date.now())));
    survivors = new Set(live.filter(processExists));
  }
}

/** Terminates a detached child and every process it owns, escalating after a bounded grace period. */
export async function terminateProcessTree(
  child: ChildProcess,
  timeoutMs = 2_000,
  forceDirectFallback = false,
  initialSignal: NodeJS.Signals = "SIGTERM",
): Promise<void> {
  if (child.exitCode !== null || child.signalCode !== null) return;
  const rootPid = child.pid;
  const exited = new Promise<void>((resolve) => child.once("exit", () => resolve()));
  const descendants = signalProcessTree(child, initialSignal, forceDirectFallback);
  const [, termSurvivors] = await Promise.all([
    Promise.race([exited, wait(timeoutMs)]),
    waitForProcessIdsExit(descendants, timeoutMs),
  ]);
  const escalationIds = new Set(termSurvivors);
  if (process.platform !== "win32" && rootPid !== undefined) {
    for (const pid of ownedProcessTreeIds(rootPid)) escalationIds.add(pid);
  }
  if ((child.exitCode === null && child.signalCode === null) || escalationIds.size > 0) {
    // The group leader can exit during the grace period while a descendant survives and forks.
    // A process-group kill closes the snapshot-to-signal race before the PID fallback verifies it.
    if (!forceDirectFallback && process.platform !== "win32" && rootPid !== undefined) {
      signalOwnedProcessGroup(rootPid, "SIGKILL");
    }
    if (child.exitCode === null && child.signalCode === null) {
      for (const pid of signalProcessTree(child, "SIGKILL", forceDirectFallback)) {
        escalationIds.add(pid);
      }
    }
    const [, killSurvivors] = await Promise.all([
      child.exitCode === null && child.signalCode === null ? exited : Promise.resolve(),
      forceKillOwnedProcessIds(rootPid, escalationIds, timeoutMs),
    ]);
    if (killSurvivors.length > 0) {
      throw new Error(
        `failed to terminate child processes ${killSurvivors.join(", ")} after SIGKILL`,
      );
    }
  }
}
