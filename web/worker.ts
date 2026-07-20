import init, { create_engine, type EngineHandle } from "./generated/voxels.js";
import type { FromWorker, InitMessage, ToWorker } from "./protocol.ts";
import { disposeWorkerEngine } from "./worker-lifecycle.ts";

const scope = self as unknown as {
  postMessage(message: FromWorker): void;
  onmessage: ((event: MessageEvent<ToWorker>) => void) | null;
  close(): void;
};

let handle: EngineHandle | null = null;
let booting: Promise<EngineHandle> | null = null;
let disposed = false;
let cursorMode = false;
let readinessTimer: ReturnType<typeof setInterval> | undefined;
let disposal: Promise<void> | null = null;
const pending: Exclude<ToWorker, InitMessage>[] = [];

function stopReadinessMonitor(): void {
  if (readinessTimer !== undefined) clearInterval(readinessTimer);
  readinessTimer = undefined;
}

function monitorReadiness(engine: EngineHandle): void {
  let previous = "";
  const update = (): void => {
    if (disposed) return;
    const [resident = 0, required = 0, playable = 0] = Array.from(engine.startup_progress());
    const key = `${resident}/${required}/${playable}`;
    if (key !== previous) {
      previous = key;
      scope.postMessage({ kind: "loading", stage: "vicinity", resident, required });
    }
    if (playable === 1) {
      stopReadinessMonitor();
      scope.postMessage({ kind: "ready" });
    }
  };
  update();
  if (!disposed && readinessTimer === undefined) readinessTimer = setInterval(update, 50);
}

function beginDisposal(): Promise<void> {
  if (disposal) return disposal;
  disposed = true;
  stopReadinessMonitor();
  pending.length = 0;
  const engine = handle;
  handle = null;
  const pendingBoot = booting;
  booting = null;
  disposal = disposeWorkerEngine(engine, pendingBoot, () => {
    scope.postMessage({ kind: "destroyed" });
    scope.close();
  });
  void disposal.catch((error: unknown) =>
    console.error(`[voxels] engine shutdown failed: ${String(error)}`),
  );
  return disposal;
}

function fail(message: string): void {
  if (disposed) return;
  scope.postMessage({ kind: "error", message });
  void beginDisposal();
}

self.addEventListener("error", (event) => {
  if (disposed) return;
  const location = event.filename
    ? `\n${event.filename}:${event.lineno || 0}:${event.colno || 0}`
    : "";
  const stack = event.error instanceof Error && event.error.stack ? `\n${event.error.stack}` : "";
  fail(`${event.message || "Uncaught engine worker error"}${location}${stack}`);
  event.preventDefault();
});

function dispatch(message: Exclude<ToWorker, InitMessage>): void {
  switch (message.kind) {
    case "automationContract":
      scope.postMessage({
        kind: "automationContract",
        requestId: message.requestId,
        value: handle?.automation_contract() ?? "",
      });
      break;
    case "input":
      {
        const next = handle?.feed_input(new Uint8Array(message.buffer)) ?? false;
        if (next !== cursorMode) {
          cursorMode = next;
          scope.postMessage({ kind: "uiMode", cursor: next });
        }
        const report = handle?.take_mission_control_copy();
        if (report !== undefined) {
          scope.postMessage({ kind: "copyMissionControl", text: report });
        }
      }
      break;
    case "resize":
      handle?.resize(message.cssWidth, message.cssHeight, message.dpr);
      break;
    case "reducedMotion":
      handle?.set_reduced_motion(message.reduced);
      break;
    case "missionControlCopyResult":
      handle?.report_mission_control_copy_result(message.copied);
      break;
    case "profile":
      handle?.start_profile(message.profileId);
      break;
    case "spectator":
      scope.postMessage({
        kind: "spectator",
        requestId: message.requestId,
        active: handle?.set_spectator(message.active) ?? false,
      });
      break;
    case "snapshot":
      scope.postMessage({
        kind: "snapshot",
        requestId: message.requestId,
        values: Array.from(handle?.snapshot() ?? []),
      });
      break;
    case "submitPlace":
      scope.postMessage({
        kind: "submitPlace",
        requestId: message.requestId,
        submitted:
          handle?.submit_place(
            message.x,
            message.y,
            message.z,
            message.materialId,
            message.shapeId,
          ) ?? false,
      });
      break;
    case "submitDig":
      scope.postMessage({
        kind: "submitDig",
        requestId: message.requestId,
        submitted: handle?.submit_dig(message.x, message.y, message.z, message.shapeId) ?? false,
      });
      break;
    case "inventory":
      scope.postMessage({
        kind: "inventory",
        requestId: message.requestId,
        values: Array.from(handle?.inventory() ?? []),
      });
      break;
    case "surfaceEditState":
      scope.postMessage({
        kind: "surfaceEditState",
        requestId: message.requestId,
        values: Array.from(handle?.surface_edit_state(message.stride, message.x, message.z) ?? []),
      });
      break;
    case "destroy":
      void beginDisposal();
      break;
  }
}

async function boot(message: InitMessage): Promise<EngineHandle> {
  scope.postMessage({ kind: "loading", stage: "wasm" });
  await init();
  scope.postMessage({ kind: "loading", stage: "world" });
  return create_engine(
    message.canvas,
    message.cssWidth,
    message.cssHeight,
    message.dpr,
    message.reducedMotion,
    message.configToml,
    [message.browserUserId, message.playerId, message.playerName],
  );
}

scope.onmessage = (event) => {
  const message = event.data;
  if (disposed) return;
  if (message.kind === "init") {
    if (booting || handle) {
      fail("engine worker received duplicate initialization");
      return;
    }
    const request = boot(message);
    booting = request;
    void request
      .then((engine) => {
        if (booting === request) booting = null;
        // Boot-time teardown awaits this same engine and owns its destruction.
        if (disposed) return;
        handle = engine;
        cursorMode = engine.ui_open();
        scope.postMessage({ kind: "uiMode", cursor: cursorMode });
        for (const queued of pending.splice(0)) dispatch(queued);
        monitorReadiness(engine);
      })
      .catch((error: unknown) => {
        if (booting === request) booting = null;
        if (disposed) return;
        fail(String(error));
      });
  } else if (!handle && message.kind !== "destroy") {
    pending.push(message);
  } else {
    dispatch(message);
  }
};
