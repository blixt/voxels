export function shouldRefreshResidency(force: boolean, anchorChanged: boolean, pendingChunks: number): boolean {
  return force || anchorChanged || pendingChunks > 0;
}

export function shouldAllowFarFieldCatchupWhileMoving(
  movementIntent: boolean,
  pendingChunks: number,
  dirtyResidentChunks: number,
  pendingFarFieldBands: number,
  frameNumber: number,
  cadenceFrames = 6,
  hasFarFieldCoverage = true,
): boolean {
  if (!movementIntent) {
    return true;
  }
  if (!hasFarFieldCoverage) {
    return cadenceFrames <= 1 || frameNumber % cadenceFrames === 0;
  }
  if (pendingChunks > 0 || dirtyResidentChunks > 0 || pendingFarFieldBands === 0) {
    return false;
  }
  return cadenceFrames <= 1 || frameNumber % cadenceFrames === 0;
}

export function shouldPumpWorldWork(
  moved: boolean,
  pendingChunks: number,
  dirtyResidentChunks: number,
  pendingFarFieldBands = 0,
): boolean {
  return moved || pendingChunks > 0 || dirtyResidentChunks > 0 || pendingFarFieldBands > 0;
}
