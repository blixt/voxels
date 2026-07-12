import "./style.css";
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
  const snapshotResolvers = new Map<number, (values: number[]) => void>();
  const debugGlobal = globalThis as typeof globalThis & {
    __VOXELS__?: { snapshot(): Promise<number[]>; profile(profileId: number): void };
  };
  debugGlobal.__VOXELS__ = {
    snapshot: () =>
      new Promise<number[]>((resolve) => {
        const requestId = nextSnapshotRequest;
        nextSnapshotRequest += 1;
        snapshotResolvers.set(requestId, resolve);
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
      fail(`The Rust engine could not start.\n${event.data.message}`);
    } else if (event.data.kind === "snapshot") {
      snapshotResolvers.get(event.data.requestId)?.(event.data.values);
      snapshotResolvers.delete(event.data.requestId);
    }
  };
  worker.onerror = (event) => fail(event.message || "The engine worker failed to load.");

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
      time: event.timeStamp,
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
  canvas.addEventListener("pointerup", (event) => {
    enqueue(point(event, INPUT_POINTER_UP), true);
  });
  canvas.addEventListener("pointercancel", (event) => {
    enqueue(point(event, INPUT_CANCEL), true);
  });
  canvas.addEventListener(
    "wheel",
    (event) => {
      event.preventDefault();
      const rect = canvas.getBoundingClientRect();
      const scale =
        event.deltaMode === WheelEvent.DOM_DELTA_LINE
          ? 16
          : event.deltaMode === 2
            ? rect.height
            : 1;
      enqueue(
        {
          kind: INPUT_WHEEL,
          code: 0,
          buttons: 0,
          x: event.clientX - rect.left,
          y: event.clientY - rect.top,
          dx: event.deltaX * scale,
          dy: event.deltaY * scale,
          time: event.timeStamp,
          flags: event.ctrlKey || event.metaKey ? 1 : 0,
        },
        true,
      );
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
    time: event.timeStamp,
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
        time: performance.now(),
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
  window.addEventListener("pagehide", () => {
    resize.disconnect();
    worker.postMessage({ kind: "destroy" });
    delete debugGlobal.__VOXELS__;
  });
}

const canvas = document.querySelector<HTMLCanvasElement>("#app");
if (canvas) start(canvas);
