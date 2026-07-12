import "./style.css";
import {
  INPUT_CANCEL,
  INPUT_KEY_DOWN,
  INPUT_KEY_UP,
  INPUT_POINTER_DOWN,
  INPUT_POINTER_MOVE,
  packInput,
  type FromWorker,
  type InputSample,
  type ToWorker,
} from "./protocol.ts";

type TypedWorker = Omit<Worker, "onmessage" | "postMessage"> & {
  onmessage: ((event: MessageEvent<FromWorker>) => void) | null;
  postMessage(message: ToWorker, transfer?: Transferable[]): void;
};

function start(canvas: HTMLCanvasElement): void {
  const fail = (message: string): void => {
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

  const worker = new Worker(new URL("./worker.ts", import.meta.url), {
    type: "module",
  }) as TypedWorker;
  let uiCursorMode = false;
  let nextSnapshotRequest = 1;
  const snapshotResolvers = new Map<
    number,
    { resolve: (values: number[]) => void; reject: (reason: Error) => void }
  >();
  const debugGlobal = globalThis as typeof globalThis & {
    __VOXELS__?: { snapshot(): Promise<number[]>; profile(profileId: number): void };
  };
  const failWorker = (message: string): void => {
    fail(message);
    worker.terminate();
    const error = new Error(message);
    for (const { reject } of snapshotResolvers.values()) reject(error);
    snapshotResolvers.clear();
    delete debugGlobal.__VOXELS__;
  };
  debugGlobal.__VOXELS__ = {
    snapshot: () =>
      new Promise<number[]>((resolve, reject) => {
        const requestId = nextSnapshotRequest;
        nextSnapshotRequest += 1;
        snapshotResolvers.set(requestId, { resolve, reject });
        worker.postMessage({ kind: "snapshot", requestId });
      }),
    profile: (profileId) => worker.postMessage({ kind: "profile", profileId }),
  };
  worker.onmessage = (event) => {
    if (event.data.kind === "uiMode") {
      uiCursorMode = event.data.cursor;
      canvas.classList.toggle("ui-cursor", uiCursorMode);
      if (uiCursorMode && document.pointerLockElement === canvas) {
        document.exitPointerLock();
      }
    } else if (event.data.kind === "error") {
      failWorker(`The Rust engine could not start.\n${event.data.message}`);
    } else if (event.data.kind === "snapshot") {
      snapshotResolvers.get(event.data.requestId)?.resolve(event.data.values);
      snapshotResolvers.delete(event.data.requestId);
    }
  };
  worker.onerror = (event) => failWorker(event.message || "The engine worker failed to load.");

  const offscreen = canvas.transferControlToOffscreen();
  const bounds = canvas.getBoundingClientRect();
  worker.postMessage(
    {
      kind: "init",
      canvas: offscreen,
      cssWidth: bounds.width,
      cssHeight: bounds.height,
      dpr: window.devicePixelRatio || 1,
      reducedMotion: window.matchMedia("(prefers-reduced-motion: reduce)").matches,
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
  const point = (event: PointerEvent, kind: number): InputSample => {
    const rect = canvas.getBoundingClientRect();
    return {
      kind,
      code: event.pointerType === "touch" ? 1 : event.pointerType === "pen" ? 2 : 0,
      buttons: event.buttons,
      x: event.clientX - rect.left,
      y: event.clientY - rect.top,
      dx: event.movementX,
      dy: event.movementY,
      flags: event.ctrlKey || event.metaKey ? 1 : 0,
    };
  };

  canvas.addEventListener("pointerdown", (event) => {
    if (event.pointerType !== "mouse") {
      canvas.setPointerCapture(event.pointerId);
    }
    if (uiCursorMode) {
      enqueue(point(event, INPUT_POINTER_DOWN), true);
      return;
    }
    if (event.pointerType === "mouse" && document.pointerLockElement !== canvas) {
      void canvas.requestPointerLock();
      return;
    }
    enqueue(point(event, INPUT_POINTER_DOWN), true);
  });
  canvas.addEventListener("contextmenu", (event) => event.preventDefault());
  canvas.addEventListener("pointermove", (event) => {
    if (event.pointerType === "mouse" && document.pointerLockElement !== canvas && !uiCursorMode) {
      return;
    }
    for (const sample of event.getCoalescedEvents?.() ?? [event]) {
      enqueue(point(sample, INPUT_POINTER_MOVE));
    }
  });
  canvas.addEventListener("pointercancel", (event) => {
    enqueue(point(event, INPUT_CANCEL), true);
  });
  canvas.addEventListener(
    "wheel",
    (event) => {
      event.preventDefault();
    },
    { passive: false },
  );

  const keyCode = (code: string): number => {
    const codes: Record<string, number> = {
      KeyW: 1,
      KeyA: 2,
      KeyS: 3,
      KeyD: 4,
      Space: 5,
      ShiftLeft: 6,
      ShiftRight: 6,
      Escape: 7,
      F3: 8,
    };
    return codes[code] ?? 0;
  };
  const keySample = (event: KeyboardEvent, kind: number): InputSample => ({
    kind,
    code: keyCode(event.code),
    buttons: 0,
    x: 0,
    y: 0,
    dx: 0,
    dy: 0,
    flags: event.repeat ? 1 : 0,
  });
  window.addEventListener("keydown", (event) => {
    if (keyCode(event.code) !== 0) {
      event.preventDefault();
      if (event.code === "F3" && document.pointerLockElement === canvas) {
        document.exitPointerLock();
      }
      enqueue(keySample(event, INPUT_KEY_DOWN), true);
    }
  });
  window.addEventListener("keyup", (event) => {
    if (keyCode(event.code) !== 0) enqueue(keySample(event, INPUT_KEY_UP), true);
  });
  const cancelInput = (): void => {
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
    if (document.pointerLockElement !== canvas) cancelInput();
  });

  const resize = new ResizeObserver(([entry]) => {
    if (!entry) return;
    worker.postMessage({
      kind: "resize",
      cssWidth: entry.contentRect.width,
      cssHeight: entry.contentRect.height,
      dpr: window.devicePixelRatio || 1,
    });
  });
  resize.observe(canvas);
  let destroyed = false;
  window.addEventListener("pagehide", (event) => {
    // A page entering the back-forward cache is frozen with its worker and must resume intact.
    // A real navigation/reload gets one explicit worker turn to close SQLite and pause the OPFS VFS.
    if (event.persisted || destroyed) return;
    destroyed = true;
    flush();
    resize.disconnect();
    const error = new Error("Voxels page closed before the snapshot completed");
    for (const { reject } of snapshotResolvers.values()) reject(error);
    snapshotResolvers.clear();
    worker.postMessage({ kind: "destroy" });
    delete debugGlobal.__VOXELS__;
  });
}

const canvas = document.querySelector<HTMLCanvasElement>("#app");
if (canvas) start(canvas);
