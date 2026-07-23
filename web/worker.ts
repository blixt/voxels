import init, {
  create_engine,
  type EngineHandle,
  type MissionControlScreenshot,
} from "./generated/voxels.js";
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
let screenshotTimer: ReturnType<typeof setInterval> | undefined;
let screenshotDeadline = 0;
let screenshotEncoding = false;
let disposal: Promise<void> | null = null;
const pending: Exclude<ToWorker, InitMessage>[] = [];

function stopReadinessMonitor(): void {
  if (readinessTimer !== undefined) clearInterval(readinessTimer);
  readinessTimer = undefined;
}

function stopScreenshotMonitor(): void {
  if (screenshotTimer !== undefined) clearInterval(screenshotTimer);
  screenshotTimer = undefined;
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
  stopScreenshotMonitor();
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

async function encodeScreenshot(capture: MissionControlScreenshot): Promise<void> {
  try {
    const width = capture.width;
    const height = capture.height;
    const rgba = capture.rgba();
    if (rgba.byteLength !== width * height * 4) {
      throw new Error("renderer returned an invalid RGBA screenshot");
    }
    const canvas = new OffscreenCanvas(width, height);
    const context = canvas.getContext("2d");
    if (!context) throw new Error("browser could not create a PNG encoding canvas");
    const pixels = new Uint8ClampedArray(rgba);
    context.putImageData(new ImageData(pixels, width, height), 0, 0);
    const blob = await canvas.convertToBlob({ type: "image/png" });
    if (blob.type !== "image/png" || blob.size < 8) {
      throw new Error("browser returned an invalid PNG screenshot");
    }
    scope.postMessage({
      kind: "downloadMissionControlScreenshot",
      blob,
      filename: capture.filename,
    });
  } catch (error) {
    console.error(`[voxels] screenshot capture failed: ${String(error)}`);
    handle?.report_mission_control_screenshot_result(false);
  } finally {
    capture.free();
    screenshotEncoding = false;
  }
}

function monitorScreenshot(): void {
  if (disposed || screenshotTimer !== undefined || screenshotEncoding) return;
  screenshotDeadline = performance.now() + 10_000;
  const update = (): void => {
    if (disposed) {
      stopScreenshotMonitor();
      return;
    }
    const capture = handle?.take_mission_control_screenshot();
    if (capture !== undefined) {
      stopScreenshotMonitor();
      screenshotEncoding = true;
      void encodeScreenshot(capture);
      return;
    }
    if (!handle?.mission_control_screenshot_pending() || performance.now() >= screenshotDeadline) {
      console.error(
        `[voxels] screenshot readback ended without pixels (pending=${String(handle?.mission_control_screenshot_pending() ?? false)})`,
      );
      stopScreenshotMonitor();
      handle?.report_mission_control_screenshot_result(false);
    }
  };
  screenshotTimer = setInterval(update, 16);
  update();
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
        if (handle?.mission_control_screenshot_pending()) monitorScreenshot();
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
    case "missionControlScreenshotResult":
      handle?.report_mission_control_screenshot_result(message.saved);
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
    case "diagnosticSky":
      scope.postMessage({
        kind: "diagnosticSky",
        requestId: message.requestId,
        active:
          handle?.set_diagnostic_sky(message.enabled, message.red, message.green, message.blue) ??
          false,
      });
      break;
    case "materialDetail":
      scope.postMessage({
        kind: "materialDetail",
        requestId: message.requestId,
        accepted: handle?.set_material_detail(message.enabled) ?? false,
      });
      break;
    case "lodBoundaries":
      scope.postMessage({
        kind: "lodBoundaries",
        requestId: message.requestId,
        accepted:
          handle?.set_lod_boundary_half_extents(new Int32Array(message.halfExtentsVoxels)) ?? false,
      });
      break;
    case "exactVolumePresented":
      scope.postMessage({
        kind: "exactVolumePresented",
        requestId: message.requestId,
        presented: handle?.exact_volume_presented(message.x, message.y, message.z) ?? false,
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
