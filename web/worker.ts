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
