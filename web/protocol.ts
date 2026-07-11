export const INPUT_RECORD_BYTES = 28;

export const INPUT_POINTER_DOWN = 0;
export const INPUT_POINTER_MOVE = 1;
export const INPUT_POINTER_UP = 2;
export const INPUT_WHEEL = 3;
export const INPUT_KEY_DOWN = 4;
export const INPUT_KEY_UP = 5;
export const INPUT_CANCEL = 6;

export interface InputSample {
  kind: number;
  code: number;
  buttons: number;
  x: number;
  y: number;
  dx: number;
  dy: number;
  time: number;
  flags: number;
}

export function packInput(samples: readonly InputSample[]): ArrayBuffer {
  const buffer = new ArrayBuffer(samples.length * INPUT_RECORD_BYTES);
  const view = new DataView(buffer);
  for (let index = 0; index < samples.length; index += 1) {
    const sample = samples[index];
    if (!sample) continue;
    const offset = index * INPUT_RECORD_BYTES;
    view.setUint8(offset, sample.kind);
    view.setUint8(offset + 1, sample.code);
    view.setUint16(offset + 2, sample.buttons, true);
    view.setFloat32(offset + 4, sample.x, true);
    view.setFloat32(offset + 8, sample.y, true);
    view.setFloat32(offset + 12, sample.dx, true);
    view.setFloat32(offset + 16, sample.dy, true);
    view.setFloat32(offset + 20, sample.time, true);
    view.setUint32(offset + 24, sample.flags, true);
  }
  return buffer;
}

export interface InitMessage {
  kind: "init";
  canvas: OffscreenCanvas;
  cssWidth: number;
  cssHeight: number;
  dpr: number;
  reducedMotion: boolean;
}

export type ToWorker =
  | InitMessage
  | { kind: "input"; buffer: ArrayBuffer }
  | { kind: "resize"; cssWidth: number; cssHeight: number; dpr: number }
  | { kind: "snapshot"; requestId: number }
  | { kind: "destroy" };

export type FromWorker =
  | { kind: "ready" }
  | { kind: "uiMode"; cursor: boolean }
  | { kind: "error"; message: string }
  | { kind: "snapshot"; requestId: number; values: number[] };
