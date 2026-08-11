import { describe, expect, it } from "vite-plus/test";
import { enqueueBeforeWorkerBoot, type QueuedWorkerMessage } from "./worker-queue.ts";

describe("worker pre-boot queue", () => {
  it("coalesces stale resizes at the latest message position", () => {
    const pending: QueuedWorkerMessage[] = [];
    enqueueBeforeWorkerBoot(pending, { kind: "profile", profileId: 1 });
    enqueueBeforeWorkerBoot(pending, { kind: "resize", cssWidth: 640, cssHeight: 480, dpr: 1 });
    enqueueBeforeWorkerBoot(pending, { kind: "clearReproduction" });
    enqueueBeforeWorkerBoot(pending, { kind: "resize", cssWidth: 1920, cssHeight: 1080, dpr: 2 });

    expect(pending).toEqual([
      { kind: "profile", profileId: 1 },
      { kind: "clearReproduction" },
      { kind: "resize", cssWidth: 1920, cssHeight: 1080, dpr: 2 },
    ]);
  });

  it("coalesces reduced-motion state independently of resize state", () => {
    const pending: QueuedWorkerMessage[] = [];
    enqueueBeforeWorkerBoot(pending, { kind: "reducedMotion", reduced: true });
    enqueueBeforeWorkerBoot(pending, { kind: "resize", cssWidth: 800, cssHeight: 600, dpr: 1 });
    enqueueBeforeWorkerBoot(pending, { kind: "reducedMotion", reduced: false });

    expect(pending).toEqual([
      { kind: "resize", cssWidth: 800, cssHeight: 600, dpr: 1 },
      { kind: "reducedMotion", reduced: false },
    ]);
  });
});
