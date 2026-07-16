export interface DisposableWorkerEngine {
  destroy(): Promise<void>;
}

/** Waits for an in-flight engine before closing its worker cleanly. */
export async function disposeWorkerEngine(
  active: DisposableWorkerEngine | null,
  booting: Promise<DisposableWorkerEngine> | null,
  close: () => void,
): Promise<void> {
  try {
    let engine = active;
    if (!engine && booting) {
      try {
        engine = await booting;
      } catch {
        // A failed boot may still have a live worker, so it must close.
      }
    }
    await engine?.destroy();
  } finally {
    close();
  }
}
