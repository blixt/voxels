export const INPUT_RECORD_BYTES = 24;

export const INPUT_POINTER_DOWN = 0;
export const INPUT_POINTER_MOVE = 1;
export const INPUT_WHEEL = 2;
export const INPUT_POINTER_UP = 3;
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
    view.setUint32(offset + 20, sample.flags, true);
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
  configToml: string;
  browserUserId: string;
  playerId: string;
  playerName: string;
}

export type ToWorker =
  | InitMessage
  | { kind: "automationContract"; requestId: number }
  | { kind: "input"; buffer: ArrayBuffer }
  | { kind: "resize"; cssWidth: number; cssHeight: number; dpr: number }
  | { kind: "reducedMotion"; reduced: boolean }
  | { kind: "missionControlCopyResult"; copied: boolean }
  | { kind: "missionControlScreenshotResult"; saved: boolean }
  | { kind: "profile"; profileId: number }
  | { kind: "spectator"; requestId: number; active: boolean }
  | {
      kind: "diagnosticSky";
      requestId: number;
      enabled: boolean;
      red: number;
      green: number;
      blue: number;
    }
  | { kind: "materialDetail"; requestId: number; enabled: boolean }
  | { kind: "lodBoundaries"; requestId: number; halfExtentsVoxels: number[] }
  | { kind: "snapshot"; requestId: number }
  | {
      kind: "submitPlace";
      requestId: number;
      x: number;
      y: number;
      z: number;
      materialId: number;
      shapeId: number;
    }
  | { kind: "submitDig"; requestId: number; x: number; y: number; z: number; shapeId: number }
  | { kind: "inventory"; requestId: number }
  | { kind: "surfaceEditState"; requestId: number; stride: number; x: number; z: number }
  | { kind: "destroy" };

export type FromWorker =
  | {
      kind: "loading";
      stage: "wasm" | "world" | "vicinity";
      resident?: number;
      required?: number;
    }
  | { kind: "ready" }
  | { kind: "destroyed" }
  | { kind: "uiMode"; cursor: boolean }
  | { kind: "copyMissionControl"; text: string }
  | { kind: "downloadMissionControlScreenshot"; blob: Blob; filename: string }
  | { kind: "error"; message: string }
  | { kind: "automationContract"; requestId: number; value: string }
  | { kind: "snapshot"; requestId: number; values: number[] }
  | { kind: "spectator"; requestId: number; active: boolean }
  | { kind: "diagnosticSky"; requestId: number; active: boolean }
  | { kind: "materialDetail"; requestId: number; accepted: boolean }
  | { kind: "lodBoundaries"; requestId: number; accepted: boolean }
  | { kind: "submitPlace"; requestId: number; submitted: boolean }
  | { kind: "submitDig"; requestId: number; submitted: boolean }
  | { kind: "inventory"; requestId: number; values: number[] }
  | { kind: "surfaceEditState"; requestId: number; values: number[] };
