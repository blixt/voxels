import type { InitMessage, ToWorker } from "./protocol.ts";

export type QueuedWorkerMessage = Exclude<ToWorker, InitMessage | { kind: "destroy" }>;

/** Retains only the latest boot-time display state while preserving event message order. */
export function enqueueBeforeWorkerBoot(
  pending: QueuedWorkerMessage[],
  message: QueuedWorkerMessage,
): void {
  if (message.kind === "resize" || message.kind === "reducedMotion") {
    for (let index = pending.length - 1; index >= 0; index -= 1) {
      if (pending[index]?.kind === message.kind) pending.splice(index, 1);
    }
  }
  pending.push(message);
}
