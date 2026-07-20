import "./style.css";
import { loadClientConfig } from "./client-config.ts";
import { writeClipboardText } from "./clipboard.ts";
import {
  type EngineAutomationApi,
  type EngineAutomationContract,
  parseAutomationContract,
} from "./automation.ts";
import { watchDevicePixelRatio } from "./display.ts";
import { terminateAfterAcknowledgement } from "./hmr-lifecycle.ts";
import { PressedKeys, WheelAccumulator, requestPointerLockSafely } from "./input.ts";
import {
  namedPlayerUrl,
  resolveBrowserPlayerSession,
  type BrowserPlayerSession,
} from "./local-player.ts";
import {
  INPUT_CANCEL,
  INPUT_KEY_DOWN,
  INPUT_KEY_UP,
  INPUT_POINTER_DOWN,
  INPUT_POINTER_MOVE,
  INPUT_POINTER_UP,
  INPUT_WHEEL,
  packInput,
  type FromWorker,
  type InputSample,
  type ToWorker,
} from "./protocol.ts";

type TypedWorker = Omit<Worker, "onmessage" | "postMessage"> & {
  onmessage: ((event: MessageEvent<FromWorker>) => void) | null;
  postMessage(message: ToWorker, transfer?: Transferable[]): void;
};

function showLoading(title: string, detail: string, progress = 0.08): void {
  const body = document.body;
  body.classList.add("loading");
  body.classList.remove("load-failed");
  body.setAttribute("aria-busy", "true");
  body.setAttribute("data-loading-title", title);
  body.setAttribute("data-loading-detail", detail);
  body.style.setProperty("--loading-progress", `${Math.max(0, Math.min(1, progress)) * 100}%`);
}

function showReady(): void {
  document.body.classList.remove("loading", "load-failed");
  document.body.setAttribute("aria-busy", "false");
  document.body.style.removeProperty("--loading-progress");
}

function showFailure(message: string): void {
  showLoading("Voxels could not start", message, 1);
  document.body.classList.add("load-failed");
}

async function start(canvas: HTMLCanvasElement): Promise<void> {
  const fail = (message: string): void => {
    canvas.classList.add("ui-cursor");
    showFailure(message);
    console.error(`[voxels] ${message}`);
  };
  if (!window.isSecureContext && location.hostname !== "localhost") {
    fail("Voxels needs a secure HTTPS connection for WebGPU.");
    return;
  }
  if (!("gpu" in navigator)) {
    fail("WebGPU is unavailable. Try a current Chrome, Edge, Firefox, or Safari release.");
    return;
  }
  if (typeof canvas.transferControlToOffscreen !== "function") {
    fail("This browser cannot transfer a canvas to the engine worker.");
    return;
  }

  let configToml: string;
  let player: BrowserPlayerSession;
  showLoading("Starting Voxels", "Loading client configuration…");
  try {
    [configToml, player] = await Promise.all([loadClientConfig(), resolveBrowserPlayerSession()]);
  } catch (error) {
    fail(`Could not load client configuration or local player.\n${String(error)}`);
    return;
  }
  if (player.playerName !== "default") document.title = `Voxels · ${player.playerName}`;

  const worker = new Worker(new URL("./worker.ts", import.meta.url), {
    type: "module",
  }) as TypedWorker;
  let uiCursorMode = false;
  let playable = false;
  let shutdownPromise: Promise<void> | undefined;
  let acknowledgeDestroyed: (() => void) | undefined;
  const destroyed = new Promise<void>((resolve) => {
    acknowledgeDestroyed = resolve;
  });
  let nextSnapshotRequest = 1;
  const snapshotResolvers = new Map<
    number,
    { resolve: (values: number[]) => void; reject: (reason: Error) => void }
  >();
  const contractResolvers = new Map<
    number,
    { resolve: (value: EngineAutomationContract) => void; reject: (reason: Error) => void }
  >();
  const editResolvers = new Map<
    number,
    { resolve: (submitted: boolean) => void; reject: (reason: Error) => void }
  >();
  const spectatorResolvers = new Map<
    number,
    { resolve: (active: boolean) => void; reject: (reason: Error) => void }
  >();
  const inventoryResolvers = new Map<
    number,
    { resolve: (values: number[]) => void; reject: (reason: Error) => void }
  >();
  const surfaceEditStateResolvers = new Map<
    number,
    { resolve: (values: number[]) => void; reject: (reason: Error) => void }
  >();
  const debugGlobal = globalThis as typeof globalThis & {
    __VOXELS__?: EngineAutomationApi;
  };
  const rejectPending = (message: string): void => {
    const error = new Error(message);
    for (const { reject } of snapshotResolvers.values()) reject(error);
    snapshotResolvers.clear();
    for (const { reject } of contractResolvers.values()) reject(error);
    contractResolvers.clear();
    for (const { reject } of editResolvers.values()) reject(error);
    editResolvers.clear();
    for (const { reject } of spectatorResolvers.values()) reject(error);
    spectatorResolvers.clear();
    for (const { reject } of inventoryResolvers.values()) reject(error);
    inventoryResolvers.clear();
    for (const { reject } of surfaceEditStateResolvers.values()) reject(error);
    surfaceEditStateResolvers.clear();
  };
  const shutdownWorker = (timeoutMs = 1_000): Promise<void> => {
    if (shutdownPromise) return shutdownPromise;
    worker.postMessage({ kind: "destroy" });
    shutdownPromise = terminateAfterAcknowledgement(destroyed, () => worker.terminate(), timeoutMs);
    return shutdownPromise;
  };
  const failWorker = (message: string, abrupt = false): void => {
    playable = false;
    fail(message);
    rejectPending(message);
    if (abrupt) {
      worker.terminate();
    } else {
      void shutdownWorker();
    }
    delete debugGlobal.__VOXELS__;
  };
  debugGlobal.__VOXELS__ = {
    contract: () =>
      new Promise<EngineAutomationContract>((resolve, reject) => {
        const requestId = nextSnapshotRequest;
        nextSnapshotRequest += 1;
        contractResolvers.set(requestId, { resolve, reject });
        worker.postMessage({ kind: "automationContract", requestId });
      }),
    snapshot: () =>
      new Promise<number[]>((resolve, reject) => {
        const requestId = nextSnapshotRequest;
        nextSnapshotRequest += 1;
        snapshotResolvers.set(requestId, { resolve, reject });
        worker.postMessage({ kind: "snapshot", requestId });
      }),
    profile: (profileId) => worker.postMessage({ kind: "profile", profileId }),
    spectator: (active) =>
      new Promise<boolean>((resolve, reject) => {
        const requestId = nextSnapshotRequest;
        nextSnapshotRequest += 1;
        spectatorResolvers.set(requestId, { resolve, reject });
        worker.postMessage({ kind: "spectator", requestId, active });
      }),
    look: (deltaX, deltaY) => {
      const buffer = packInput([
        {
          kind: INPUT_POINTER_MOVE,
          code: 0,
          buttons: 0,
          x: 0,
          y: 0,
          dx: deltaX,
          dy: deltaY,
          flags: 0,
        },
      ]);
      worker.postMessage({ kind: "input", buffer }, [buffer]);
    },
    submitPlace: (x, y, z, materialId, shape) =>
      new Promise<boolean>((resolve, reject) => {
        const requestId = nextSnapshotRequest;
        nextSnapshotRequest += 1;
        editResolvers.set(requestId, { resolve, reject });
        worker.postMessage({
          kind: "submitPlace",
          requestId,
          x,
          y,
          z,
          materialId,
          shapeId: shape === "sphere" ? 0 : 1,
        });
      }),
    submitDig: (x, y, z, shape) =>
      new Promise<boolean>((resolve, reject) => {
        const requestId = nextSnapshotRequest;
        nextSnapshotRequest += 1;
        editResolvers.set(requestId, { resolve, reject });
        worker.postMessage({
          kind: "submitDig",
          requestId,
          x,
          y,
          z,
          shapeId: shape === "sphere" ? 0 : 1,
        });
      }),
    inventory: () =>
      new Promise<number[]>((resolve, reject) => {
        const requestId = nextSnapshotRequest;
        nextSnapshotRequest += 1;
        inventoryResolvers.set(requestId, { resolve, reject });
        worker.postMessage({ kind: "inventory", requestId });
      }),
    surfaceEditState: (stride, x, z) =>
      new Promise<number[]>((resolve, reject) => {
        const requestId = nextSnapshotRequest;
        nextSnapshotRequest += 1;
        surfaceEditStateResolvers.set(requestId, { resolve, reject });
        worker.postMessage({ kind: "surfaceEditState", requestId, stride, x, z });
      }),
    player,
    playerUrl: (name) => namedPlayerUrl(name).href,
  };
  worker.onmessage = (event) => {
    if (event.data.kind === "loading") {
      if (event.data.stage === "wasm") {
        showLoading("Starting native engine", "Loading the Rust/WebAssembly runtime…", 0.12);
      } else if (event.data.stage === "world") {
        showLoading("Connecting to world", "Opening the native world service and renderer…", 0.22);
      } else {
        const resident = event.data.resident ?? 0;
        const required = event.data.required ?? 0;
        const fraction = required > 0 ? resident / required : 0;
        showLoading(
          "Loading nearby world",
          required > 0
            ? `${resident} of ${required} collision-safe chunks ready`
            : "Prioritizing the player vicinity…",
          0.25 + fraction * 0.75,
        );
      }
    } else if (event.data.kind === "ready") {
      playable = true;
      showReady();
    } else if (event.data.kind === "destroyed") {
      acknowledgeDestroyed?.();
      acknowledgeDestroyed = undefined;
    } else if (event.data.kind === "uiMode") {
      uiCursorMode = event.data.cursor;
      canvas.classList.toggle("ui-cursor", uiCursorMode);
      if (uiCursorMode && document.pointerLockElement === canvas) {
        document.exitPointerLock();
      }
    } else if (event.data.kind === "copyMissionControl") {
      void writeClipboardText(navigator.clipboard, event.data.text).then((copied) => {
        worker.postMessage({ kind: "missionControlCopyResult", copied });
      });
    } else if (event.data.kind === "error") {
      failWorker(`The Rust engine could not start.\n${event.data.message}`);
    } else if (event.data.kind === "automationContract") {
      try {
        contractResolvers
          .get(event.data.requestId)
          ?.resolve(parseAutomationContract(event.data.value));
      } catch (error) {
        contractResolvers
          .get(event.data.requestId)
          ?.reject(error instanceof Error ? error : new Error(String(error)));
      }
      contractResolvers.delete(event.data.requestId);
    } else if (event.data.kind === "snapshot") {
      snapshotResolvers.get(event.data.requestId)?.resolve(event.data.values);
      snapshotResolvers.delete(event.data.requestId);
    } else if (event.data.kind === "spectator") {
      spectatorResolvers.get(event.data.requestId)?.resolve(event.data.active);
      spectatorResolvers.delete(event.data.requestId);
    } else if (event.data.kind === "submitPlace") {
      editResolvers.get(event.data.requestId)?.resolve(event.data.submitted);
      editResolvers.delete(event.data.requestId);
    } else if (event.data.kind === "submitDig") {
      editResolvers.get(event.data.requestId)?.resolve(event.data.submitted);
      editResolvers.delete(event.data.requestId);
    } else if (event.data.kind === "inventory") {
      inventoryResolvers.get(event.data.requestId)?.resolve(event.data.values);
      inventoryResolvers.delete(event.data.requestId);
    } else if (event.data.kind === "surfaceEditState") {
      surfaceEditStateResolvers.get(event.data.requestId)?.resolve(event.data.values);
      surfaceEditStateResolvers.delete(event.data.requestId);
    }
  };
  worker.onerror = (event) => {
    const location = event.filename
      ? `\n${event.filename}:${event.lineno || 0}:${event.colno || 0}`
      : "";
    const stack = event.error instanceof Error && event.error.stack ? `\n${event.error.stack}` : "";
    failWorker(`${event.message || "The engine worker failed to load."}${location}${stack}`, true);
  };

  import.meta.hot?.on("vite:beforeFullReload", async () => {
    playable = false;
    showLoading("Reloading Voxels", "Saving local state and restarting the native engine…", 0.1);
    await shutdownWorker();
  });
  import.meta.hot?.on("voxels:before-world-restart", async () => {
    playable = false;
    showLoading("Updating native world", "Saving player state before restarting the server…", 0.1);
    await shutdownWorker();
  });

  const offscreen = canvas.transferControlToOffscreen();
  const bounds = canvas.getBoundingClientRect();
  const reducedMotion = window.matchMedia("(prefers-reduced-motion: reduce)");
  worker.postMessage(
    {
      kind: "init",
      canvas: offscreen,
      cssWidth: bounds.width,
      cssHeight: bounds.height,
      dpr: window.devicePixelRatio || 1,
      reducedMotion: reducedMotion.matches,
      configToml,
      browserUserId: player.browserUserId,
      playerId: player.playerId,
      playerName: player.playerName,
    },
    [offscreen],
  );

  let pending: InputSample[] = [];
  let scheduled = false;
  const flush = (): void => {
    if (pending.length === 0) return;
    const buffer = packInput(pending);
    pending = [];
    worker.postMessage({ kind: "input", buffer }, [buffer]);
  };
  const enqueue = (sample: InputSample, immediate = false): void => {
    if (!playable && sample.kind !== INPUT_CANCEL) return;
    pending.push(sample);
    if (immediate) {
      flush();
      return;
    }
    if (!scheduled) {
      scheduled = true;
      queueMicrotask(() => {
        scheduled = false;
        flush();
      });
    }
  };
  const point = (event: PointerEvent, kind: number, rect?: DOMRectReadOnly): InputSample => {
    return {
      kind,
      code: event.pointerType === "touch" ? 1 : event.pointerType === "pen" ? 2 : 0,
      buttons: event.buttons,
      x: rect ? event.clientX - rect.left : 0,
      y: rect ? event.clientY - rect.top : 0,
      dx: event.movementX,
      dy: event.movementY,
      flags: event.ctrlKey || event.metaKey ? 1 : 0,
    };
  };

  canvas.addEventListener("pointerdown", (event) => {
    if (!playable) return;
    if (event.pointerType !== "mouse") {
      canvas.setPointerCapture(event.pointerId);
    }
    if (uiCursorMode) {
      enqueue(point(event, INPUT_POINTER_DOWN, canvas.getBoundingClientRect()), true);
      return;
    }
    if (event.pointerType === "mouse" && document.pointerLockElement !== canvas) {
      void requestPointerLockSafely(
        () => canvas.requestPointerLock(),
        (error) => console.warn(`[voxels] Pointer lock request failed; click to retry.`, error),
      );
      return;
    }
    enqueue(point(event, INPUT_POINTER_DOWN, canvas.getBoundingClientRect()), true);
  });
  canvas.addEventListener("contextmenu", (event) => event.preventDefault());
  canvas.addEventListener("pointermove", (event) => {
    if (event.pointerType === "mouse" && document.pointerLockElement !== canvas && !uiCursorMode) {
      return;
    }
    // Pointer-locked gameplay consumes only movement deltas. Read layout once for an unlocked UI
    // event, rather than once per coalesced sample, and skip the DOM read entirely while looking.
    const rect =
      uiCursorMode || document.pointerLockElement !== canvas
        ? canvas.getBoundingClientRect()
        : undefined;
    for (const sample of event.getCoalescedEvents?.() ?? [event]) {
      enqueue(point(sample, INPUT_POINTER_MOVE, rect));
    }
  });
  canvas.addEventListener("pointercancel", (event) => {
    enqueue(point(event, INPUT_CANCEL), true);
  });
  canvas.addEventListener("pointerup", (event) => {
    enqueue(point(event, INPUT_POINTER_UP, canvas.getBoundingClientRect()), true);
  });
  const wheelAccumulator = new WheelAccumulator();
  canvas.addEventListener(
    "wheel",
    (event) => {
      event.preventDefault();
      for (const direction of wheelAccumulator.consume(
        event.deltaY,
        event.deltaMode,
        window.innerHeight,
      )) {
        enqueue(
          {
            kind: INPUT_WHEEL,
            code: 0,
            buttons: 0,
            x: 0,
            y: 0,
            dx: 0,
            dy: direction,
            flags: 0,
          },
          true,
        );
      }
    },
    { passive: false },
  );

  const pressedKeys = new PressedKeys();
  const keySample = (event: KeyboardEvent, kind: number, code: number): InputSample => ({
    kind,
    code,
    buttons: 0,
    x: 0,
    y: 0,
    dx: 0,
    dy: 0,
    flags: event.repeat ? 1 : 0,
  });
  window.addEventListener("keydown", (event) => {
    const code = pressedKeys.keyDown(event.code);
    if (code !== 0) {
      event.preventDefault();
      if (event.code === "F3" && document.pointerLockElement === canvas) {
        document.exitPointerLock();
      }
      enqueue(keySample(event, INPUT_KEY_DOWN, code), true);
    }
  });
  window.addEventListener("keyup", (event) => {
    const code = pressedKeys.keyUp(event.code);
    if (code !== 0) enqueue(keySample(event, INPUT_KEY_UP, code), true);
  });
  const cancelInput = (): void => {
    pressedKeys.clear();
    wheelAccumulator.clear();
    enqueue(
      {
        kind: INPUT_CANCEL,
        code: 0,
        buttons: 0,
        x: 0,
        y: 0,
        dx: 0,
        dy: 0,
        flags: 0,
      },
      true,
    );
  };
  window.addEventListener("blur", cancelInput);
  document.addEventListener("pointerlockchange", () => {
    if (document.pointerLockElement === canvas) {
      canvas.classList.toggle("ui-cursor", uiCursorMode);
    } else {
      cancelInput();
    }
  });

  const postResize = (cssWidth: number, cssHeight: number): void => {
    worker.postMessage({
      kind: "resize",
      cssWidth,
      cssHeight,
      dpr: window.devicePixelRatio || 1,
    });
  };
  const resize = new ResizeObserver(([entry]) => {
    if (!entry) return;
    postResize(entry.contentRect.width, entry.contentRect.height);
  });
  resize.observe(canvas);
  const stopWatchingPixelRatio = watchDevicePixelRatio(
    () => window.devicePixelRatio || 1,
    (query) => window.matchMedia(query),
    () => {
      const bounds = canvas.getBoundingClientRect();
      postResize(bounds.width, bounds.height);
    },
  );
  const handleReducedMotionChange = (event: MediaQueryListEvent): void => {
    worker.postMessage({ kind: "reducedMotion", reduced: event.matches });
  };
  reducedMotion.addEventListener("change", handleReducedMotionChange);
  let pageClosing = false;
  window.addEventListener("pagehide", (event) => {
    // A page entering the back-forward cache is frozen with its worker and must resume intact.
    // A real navigation/reload gets one explicit worker turn to close its native connections.
    if (event.persisted || pageClosing) return;
    pageClosing = true;
    playable = false;
    flush();
    resize.disconnect();
    stopWatchingPixelRatio();
    reducedMotion.removeEventListener("change", handleReducedMotionChange);
    rejectPending("Voxels page closed before the request completed");
    void shutdownWorker();
    delete debugGlobal.__VOXELS__;
  });
}

const canvas = document.querySelector<HTMLCanvasElement>("#app");
if (canvas) {
  void start(canvas);
} else {
  showFailure("The application canvas is missing.");
}
