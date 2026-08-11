import type { InitMessage } from "./protocol.ts";

export interface InitializableEngineWorker {
  postMessage(message: InitMessage, transfer: Transferable[]): void;
  terminate(): void;
}

/** Creates and initializes a worker as one transaction, cleaning up every partial worker. */
export function initializeEngineWorker<WorkerType extends InitializableEngineWorker>(
  createWorker: () => WorkerType,
  transferCanvas: () => OffscreenCanvas,
  init: Omit<InitMessage, "canvas">,
): WorkerType {
  const worker = createWorker();
  try {
    const canvas = transferCanvas();
    worker.postMessage({ ...init, canvas }, [canvas]);
    return worker;
  } catch (error) {
    worker.terminate();
    throw error;
  }
}
