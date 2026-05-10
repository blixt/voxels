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
