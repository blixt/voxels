export function shouldRefreshResidency(force: boolean, anchorChanged: boolean, pendingChunks: number): boolean {
  return force || anchorChanged || pendingChunks > 0;
}

export function shouldPumpWorldWork(
  moved: boolean,
  pendingChunks: number,
  dirtyResidentChunks: number,
  pendingLodChunks = 0,
): boolean {
  return moved || pendingChunks > 0 || dirtyResidentChunks > 0 || pendingLodChunks > 0;
}

export interface MovingLodUpdateState {
  movementActive: boolean;
  frameNumber: number;
  intervalFrames: number;
  pendingChunks: number;
  dirtyResidentChunks: number;
  pendingLodChunks: number;
}

export function shouldRunMovingLodUpdate(state: MovingLodUpdateState): boolean {
  if (!state.movementActive || state.intervalFrames <= 0 || state.frameNumber % state.intervalFrames !== 0) {
    return false;
  }
  return state.pendingLodChunks > 0 || (state.pendingChunks === 0 && state.dirtyResidentChunks === 0);
}
