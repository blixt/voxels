export interface PendingWorkerSlot {
  pendingCount: number;
}

export function chooseLeastPendingWorkerIndex(slots: readonly PendingWorkerSlot[]): number {
  let bestWorkerIndex = 0;
  let bestPendingCount = slots[0]?.pendingCount ?? 0;
  for (let index = 1; index < slots.length; index += 1) {
    const pendingCount = slots[index]!.pendingCount;
    if (pendingCount < bestPendingCount) {
      bestPendingCount = pendingCount;
      bestWorkerIndex = index;
    }
  }
  return bestWorkerIndex;
}

export function drainArray<T>(items: T[], maxCount = Number.POSITIVE_INFINITY): T[] {
  if (items.length === 0) {
    return [];
  }
  const drainCount = Number.isFinite(maxCount)
    ? Math.max(0, Math.min(items.length, Math.floor(maxCount)))
    : items.length;
  return drainCount === 0 ? [] : items.splice(0, drainCount);
}
