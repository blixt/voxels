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
const pending: Exclude<ToWorker, InitMessage>[] = [];

self.addEventListener("error", (event) => {
  if (disposed) return;
  disposed = true;
  pending.length = 0;
  const location = event.filename
    ? `\n${event.filename}:${event.lineno || 0}:${event.colno || 0}`
    : "";
  const stack = event.error instanceof Error && event.error.stack ? `\n${event.error.stack}` : "";
  scope.postMessage({
    kind: "error",
    message: `${event.message || "Uncaught engine worker error"}${location}${stack}`,
  });
  event.preventDefault();
  scope.close();
});

function dispatch(message: Exclude<ToWorker, InitMessage>): void {
  switch (message.kind) {
    case "input":
      {
        const next = handle?.feed_input(new Uint8Array(message.buffer)) ?? false;
        if (next !== cursorMode) {
          cursorMode = next;
          scope.postMessage({ kind: "uiMode", cursor: next });
        }
      }
      break;
    case "resize":
      handle?.resize(message.cssWidth, message.cssHeight, message.dpr);
      break;
    case "reducedMotion":
      handle?.set_reduced_motion(message.reduced);
      break;
    case "profile":
      handle?.start_profile(message.profileId);
      break;
    case "snapshot":
      scope.postMessage({
        kind: "snapshot",
        requestId: message.requestId,
        values: Array.from(handle?.snapshot() ?? []),
      });
      break;
    case "relocateCamera":
      scope.postMessage({
        kind: "relocateCamera",
        requestId: message.requestId,
        relocated: handle?.relocate_camera(message.x, message.y, message.z) ?? false,
      });
      break;
    case "submitEdit":
      scope.postMessage({
        kind: "submitEdit",
        requestId: message.requestId,
        submitted:
          handle?.submit_edit(message.x, message.y, message.z, message.materialId) ?? false,
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
      disposed = true;
      pending.length = 0;
      {
        const engine = handle;
        handle = null;
        const pendingBoot = booting;
        booting = null;
        void disposeWorkerEngine(engine, pendingBoot, () => scope.close()).catch((error: unknown) =>
          console.error(`[voxels] engine shutdown failed: ${String(error)}`),
        );
      }
      break;
  }
}

async function boot(message: InitMessage): Promise<EngineHandle> {
  await init();
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
    const request = boot(message);
    booting = request;
    void request
      .then((engine) => {
        if (booting === request) booting = null;
        // Boot-time teardown awaits this same engine and owns its destruction.
        if (disposed) return;
        handle = engine;
        for (const queued of pending.splice(0)) dispatch(queued);
      })
      .catch((error: unknown) => {
        if (booting === request) booting = null;
        if (disposed) return;
        disposed = true;
        pending.length = 0;
        scope.postMessage({ kind: "error", message: String(error) });
        scope.close();
      });
  } else if (!handle && message.kind !== "destroy") {
    pending.push(message);
  } else {
    dispatch(message);
  }
};
