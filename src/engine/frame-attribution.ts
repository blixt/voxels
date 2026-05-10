export interface FrameAttributionCauseInput {
  wallMs: number;
  gameplayMs: number;
  movementMs: number;
  streamMs: number;
  meshMs: number;
  lodMs: number;
  renderCpuMs: number;
  renderSyncMs: number;
  renderUploadMs: number;
}

export function classifyFrameAttributionCause(input: FrameAttributionCauseInput): string {
  const candidates: Array<[cause: string, ms: number]> = [
    ["stream", input.streamMs],
    ["mesh", input.meshMs],
    ["LOD", input.lodMs],
    ["render", input.renderCpuMs],
    ["GPU upload", input.renderUploadMs],
    ["GPU sync", input.renderSyncMs],
    ["movement", input.movementMs],
  ];
  candidates.sort((left, right) => right[1] - left[1]);
  const [cause, ms] = candidates[0] ?? ["none", 0];
  if (ms <= 0.05) {
    return "none";
  }

  const measuredWorkMs = Math.max(0, input.gameplayMs) + Math.max(0, input.renderCpuMs);
  const unaccountedWallMs = Math.max(0, input.wallMs - measuredWorkMs);
  if (input.wallMs >= 50 && unaccountedWallMs >= 16 && measuredWorkMs < input.wallMs * 0.5) {
    return "browser or idle";
  }
  return cause;
}
