import init, {
  create_engine,
  type EngineHandle,
  type MissionControlScreenshot,
} from "./generated/voxels.js";
import { embedPngBinary, embedPngText } from "./png-metadata.ts";
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
const STARTUP_PROGRESS_VERSION = 6;
const STARTUP_PROGRESS_WORDS = 72;
const STARTUP_SCHEMA_MISMATCH_TIMEOUT_MS = 5_000;

function bitmaskLabels(flags: number, labels: ReadonlyArray<readonly [number, string]>): string {
  if (flags === 0) return "none";
  const known = labels.filter(([flag]) => (flags & flag) !== 0).map(([, label]) => label);
  const knownMask = labels.reduce((mask, [flag]) => mask | flag, 0);
  const unknown = flags & ~knownMask;
  if (unknown !== 0) known.push(`unknown-0x${unknown.toString(16)}`);
  return known.join(",");
}

function gpuMatchFailureLabels(flags: number): string {
  return bitmaskLabels(flags, [
    [1, "feedback-missing"],
    [2, "oracle-cut-missing"],
    [4, "invalid-submission"],
    [8, "ownership-overflow"],
    [16, "fingerprint"],
    [32, "ownerless"],
    [64, "encoded-count"],
    [128, "cpu-overflow"],
    [256, "selected-pages"],
    [512, "candidate-encode-failed"],
  ]);
}

function uploadFailureLabel(kind: number): string {
  return (
    [
      "none",
      "hierarchy",
      "representation",
      "surface-cluster",
      "triangle-cluster",
      "page-too-large",
      "source-capacity",
      "gpu-snapshot",
      "snapshot-capacity",
      "no-renderable-cut",
      "selected-page-missing",
      "gpu-not-certified",
      "incomplete-partition",
      "source-working-set",
    ][kind] ?? `unknown-${kind}`
  );
}

function pageFailureLabel(kind: number): string {
  return ["none", "unavailable", "stale-revision", "generation"][kind] ?? `unknown-${kind}`;
}

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
  let lastDetailedPost = Number.NEGATIVE_INFINITY;
  let schemaMismatch = "";
  let schemaMismatchSince: number | undefined;
  const update = (): void => {
    if (disposed) return;
    const progress = Array.from(engine.startup_progress());
    if (
      progress.length !== STARTUP_PROGRESS_WORDS ||
      progress[0] !== STARTUP_PROGRESS_VERSION ||
      progress[1] !== STARTUP_PROGRESS_WORDS
    ) {
      const mismatch = `Engine update in progress (startup schema v${progress[0] ?? "missing"}/${progress.length}, expected v${STARTUP_PROGRESS_VERSION}/${STARTUP_PROGRESS_WORDS}).`;
      if (mismatch !== schemaMismatch) {
        schemaMismatch = mismatch;
        schemaMismatchSince ??= performance.now();
        scope.postMessage({
          kind: "loading",
          stage: "vicinity",
          resident: 0,
          required: 0,
          detail: mismatch,
        });
      }
      if (
        schemaMismatchSince !== undefined &&
        performance.now() - schemaMismatchSince >= STARTUP_SCHEMA_MISMATCH_TIMEOUT_MS
      ) {
        stopReadinessMonitor();
        scope.postMessage({
          kind: "error",
          message: `${mismatch} Reload the page after the engine rebuild completes.`,
        });
      }
      return;
    }
    schemaMismatch = "";
    schemaMismatchSince = undefined;
    const [
      ,
      ,
      resident = 0,
      required = 0,
      playable = 0,
      terrainReady = 0,
      gpuMatchesCpu = 0,
      cpuSelected = 0,
      cpuRequested = 0,
      cpuRefinementRoots = 0,
      cpuOwnerless = 0,
      cpuDiscontinuities = 0,
      gpuSelected = 0,
      gpuEncodedPages = 0,
      gpuOwnerless = 0,
      gpuEncodingOverflow = 0,
      presentedMatchesCut = 0,
      gpuMatchFailures = 0,
      streamPending = 0,
      streamInFlight = 0,
      streamFailed = 0,
      directoryInFlight = 0,
      columns = 0,
      columnInFlight = 0,
      columnRevisionFloors = 0,
      regionRevisionFloors = 0,
      currentColumnKnown = 0,
      currentColumnRoots = 0,
      currentColumnRegisteredRoots = 0,
      columnAccepted = 0,
      columnSubmitDeferred = 0,
      columnPreempted = 0,
      columnTimedOut = 0,
      columnOtherFailed = 0,
      directoryAccepted = 0,
      directorySubmitDeferred = 0,
      directoryPreempted = 0,
      directoryTimedOut = 0,
      directoryOtherFailed = 0,
      pageSubmitDeferred = 0,
      pagePreempted = 0,
      pageTimedOut = 0,
      pageOtherFailed = 0,
      pageUnavailable = 0,
      pageStaleRevision = 0,
      pageGenerationFailed = 0,
      pageUploadFailed = 0,
      lastPageUploadFailureKind = 0,
      lastPageFailureKind = 0,
      lastPageFailureLevel = 0,
      lastPageFailureX = 0,
      lastPageFailureY = 0,
      lastPageFailureZ = 0,
      streamUsefulKiB = 0,
      cachePages = 0,
      residentPages = 0,
      gpuAllocatedMiB = 0,
      gpuCapacityMiB = 0,
      publishedPages = 0,
      publishedExactPages = 0,
      publishedDiscontinuities = 0,
      exactDomainComplete = 0,
      exactDomainRequiredLeaves = 0,
      exactDomainCurrentCoverage = 0,
      exactDomainFingerprintLow = 0,
      exactDomainFingerprintHigh = 0,
      exactCoreComplete = 0,
      exactCoreRequiredLeaves = 0,
      exactCoreCurrentCoverage = 0,
      exactPredictionComplete = 0,
      exactPredictionRequiredLeaves = 0,
      exactPredictionCurrentCoverage = 0,
    ] = progress;
    const key = [
      resident,
      required,
      playable,
      terrainReady,
      gpuMatchesCpu,
      cpuSelected,
      cpuRequested,
      cpuRefinementRoots,
      cpuOwnerless,
      cpuDiscontinuities,
      gpuSelected,
      gpuEncodedPages,
      gpuOwnerless,
      gpuEncodingOverflow,
      presentedMatchesCut,
      gpuMatchFailures,
      streamPending,
      streamInFlight,
      streamFailed,
      directoryInFlight,
      columns,
      columnInFlight,
      columnRevisionFloors,
      regionRevisionFloors,
      currentColumnKnown,
      currentColumnRoots,
      currentColumnRegisteredRoots,
      columnAccepted,
      columnSubmitDeferred,
      columnPreempted,
      columnTimedOut,
      columnOtherFailed,
      directoryAccepted,
      directorySubmitDeferred,
      directoryPreempted,
      directoryTimedOut,
      directoryOtherFailed,
      pageSubmitDeferred,
      pagePreempted,
      pageTimedOut,
      pageOtherFailed,
      pageUnavailable,
      pageStaleRevision,
      pageGenerationFailed,
      pageUploadFailed,
      lastPageUploadFailureKind,
      lastPageFailureKind,
      lastPageFailureLevel,
      lastPageFailureX,
      lastPageFailureY,
      lastPageFailureZ,
      streamUsefulKiB,
      cachePages,
      residentPages,
      gpuAllocatedMiB,
      gpuCapacityMiB,
      publishedPages,
      publishedExactPages,
      publishedDiscontinuities,
      exactDomainComplete,
      exactDomainRequiredLeaves,
      exactDomainCurrentCoverage,
      exactDomainFingerprintLow,
      exactDomainFingerprintHigh,
      exactCoreComplete,
      exactCoreRequiredLeaves,
      exactCoreCurrentCoverage,
      exactPredictionComplete,
      exactPredictionRequiredLeaves,
      exactPredictionCurrentCoverage,
    ].join("/");
    if (key !== previous) {
      const now = performance.now();
      const detailed = resident >= required && playable === 0;
      if (detailed && now - lastDetailedPost < 250) return;
      previous = key;
      if (detailed) lastDetailedPost = now;
      const detail = detailed
        ? `Terrain cut: CPU ${cpuSelected} pages/${cpuRequested} requested/${cpuRefinementRoots} refinement roots/${cpuOwnerless} ownerless/${cpuDiscontinuities} skipped-level edges; GPU ${gpuSelected} selected/${gpuEncodedPages} encoded pages/${gpuOwnerless} ownerless/encoding overflow ${gpuEncodingOverflow}; candidate match ${gpuMatchesCpu === 1 ? "yes" : `no (${gpuMatchFailureLabels(gpuMatchFailures)})`}, presented cut match ${presentedMatchesCut === 1 ? "yes" : "no"}; exact guarantee ${exactDomainComplete === 1 ? "full prediction" : "current core"} ${exactDomainCurrentCoverage}/${exactDomainRequiredLeaves} leaves fingerprint ${exactDomainFingerprintHigh.toString(16).padStart(8, "0")}${exactDomainFingerprintLow.toString(16).padStart(8, "0")}; core ${exactCoreComplete === 1 ? "complete" : "invalid"} ${exactCoreCurrentCoverage}/${exactCoreRequiredLeaves}; prediction ${exactPredictionComplete === 1 ? "complete" : "truncated"} ${exactPredictionCurrentCoverage}/${exactPredictionRequiredLeaves}; discovery ${columns} columns/${columnInFlight} in flight/${columnRevisionFloors} column revision floors/${regionRevisionFloors} region revision floors/current ${currentColumnKnown === 1 ? `${currentColumnRegisteredRoots}/${currentColumnRoots} roots registered` : "column unknown"}, flow ${columnAccepted} accepted/${columnSubmitDeferred} deferred/${columnPreempted} preempted/${columnTimedOut} timed out/${columnOtherFailed} errors; directories ${directoryAccepted} accepted/${directorySubmitDeferred} deferred/${directoryPreempted} preempted/${directoryTimedOut} timed out/${directoryOtherFailed} errors/${directoryInFlight} in flight; stream ${streamPending} pending/${streamInFlight} in flight/${streamFailed} failed/${streamUsefulKiB} KiB useful, cache ${cachePages}, resident ${residentPages}, GPU ${gpuAllocatedMiB}/${gpuCapacityMiB} MiB; page flow ${pageSubmitDeferred} deferred/${pagePreempted} preempted/${pageTimedOut} timed out/${pageOtherFailed} transport errors; products ${pageUnavailable} unavailable/${pageStaleRevision} stale/${pageGenerationFailed} generation/${pageUploadFailed} upload (last ${uploadFailureLabel(lastPageUploadFailureKind)})${lastPageFailureKind === 0 ? "" : `, last product ${pageFailureLabel(lastPageFailureKind)}@L${lastPageFailureLevel}(${lastPageFailureX | 0},${lastPageFailureY | 0},${lastPageFailureZ | 0})`}; published ${publishedPages} pages (${publishedExactPages} exact, ${publishedDiscontinuities} skipped-level edges), terrain ${terrainReady === 1 ? "ready" : "pending"}.`
        : undefined;
      scope.postMessage({ kind: "loading", stage: "vicinity", resident, required, detail });
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
    const terrainDiagnostic = capture.terrain_diagnostic_u32x5();
    const metadata = JSON.parse(capture.metadata) as {
      attachments?: { terrainPixelOwnership?: { populated?: boolean } };
    };
    const diagnosticPopulated = metadata.attachments?.terrainPixelOwnership?.populated === true;
    if (terrainDiagnostic.byteLength !== (diagnosticPopulated ? width * height * 20 : 0)) {
      throw new Error("renderer returned an invalid u32x5 terrain diagnostic attachment");
    }
    const canvas = new OffscreenCanvas(width, height);
    const context = canvas.getContext("2d");
    if (!context) throw new Error("browser could not create a PNG encoding canvas");
    const pixels = new Uint8ClampedArray(rgba);
    context.putImageData(new ImageData(pixels, width, height), 0, 0);
    const browserPng = await canvas.convertToBlob({ type: "image/png" });
    if (browserPng.type !== "image/png" || browserPng.size < 8) {
      throw new Error("browser returned an invalid PNG screenshot");
    }
    const metadataPng = embedPngText(
      new Uint8Array(await browserPng.arrayBuffer()),
      "voxels.reproduction",
      capture.metadata,
    );
    let png = metadataPng;
    if (diagnosticPopulated) {
      const compressor = new CompressionStream("deflate");
      const compressedDiagnostic = new Uint8Array(
        await new Response(
          new Blob([terrainDiagnostic.slice().buffer as ArrayBuffer])
            .stream()
            .pipeThrough(compressor),
        ).arrayBuffer(),
      );
      // Big-endian framing makes the attachment self-describing without relying on JS typed-array
      // host endianness. The compressed payload itself expands to the little-endian RGBA32Uint
      // rows described by voxels.reproduction.v2.
      const headerBytes = 20;
      const diagnosticPayload = new Uint8Array(headerBytes + compressedDiagnostic.byteLength);
      diagnosticPayload.set([0x56, 0x54, 0x50, 0x31], 0); // "VTP1"
      const header = new DataView(diagnosticPayload.buffer);
      header.setUint16(4, 1);
      header.setUint16(6, 5);
      header.setUint32(8, width);
      header.setUint32(12, height);
      header.setUint32(16, terrainDiagnostic.byteLength);
      diagnosticPayload.set(compressedDiagnostic, headerBytes);
      png = embedPngBinary(metadataPng, "vpDI", diagnosticPayload);
    }
    const blob = new Blob([png.slice().buffer as ArrayBuffer], { type: "image/png" });
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
    // A second F2 press can arrive while the prior PNG is still being compressed. The renderer
    // accepts that next GPU capture, but `monitorScreenshot` deliberately does not start two
    // encoders at once. Resume it here so the accepted capture cannot remain pending forever
    // merely because no unrelated input event follows the key press.
    if (handle?.mission_control_screenshot_pending()) monitorScreenshot();
  }
}

function monitorScreenshot(): void {
  if (disposed || screenshotTimer !== undefined || screenshotEncoding) return;
  // Full-resolution diagnostic captures copy five integer/depth channels in addition to color.
  // Slow adapters and software WebGPU can legitimately take minutes without the readback being
  // stuck; retain a finite bound while allowing the requested full-resolution capture to finish.
  screenshotDeadline = performance.now() + 180_000;
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
    case "applyReproduction":
      scope.postMessage({
        kind: "applyReproduction",
        requestId: message.requestId,
        error: handle?.apply_reproduction(message.metadata) ?? "engine is unavailable",
      });
      break;
    case "clearReproduction":
      handle?.clear_reproduction();
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
    case "geometrySourceDebug":
      scope.postMessage({
        kind: "geometrySourceDebug",
        requestId: message.requestId,
        accepted: handle?.set_geometry_source_debug(message.enabled) ?? false,
      });
      break;
    case "materialDetail":
      scope.postMessage({
        kind: "materialDetail",
        requestId: message.requestId,
        accepted: handle?.set_material_detail(message.enabled) ?? false,
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
