import { describe, expect, it } from "vite-plus/test";
import type { InitMessage } from "./protocol.ts";
import { initializeEngineWorker, type InitializableEngineWorker } from "./engine-worker-startup.ts";

const init: Omit<InitMessage, "canvas"> = {
  kind: "init",
  cssWidth: 800,
  cssHeight: 600,
  dpr: 2,
  reducedMotion: false,
  configToml: "schema = 1",
  browserUserId: "browser-user",
  playerId: "player",
  playerName: "default",
};

describe("engine worker startup", () => {
  it("posts the transferred canvas exactly once", () => {
    const canvas = {} as OffscreenCanvas;
    const posts: Array<{ message: InitMessage; transfer: Transferable[] }> = [];
    let terminated = 0;
    const worker: InitializableEngineWorker = {
      postMessage: (message, transfer) => posts.push({ message, transfer }),
      terminate: () => {
        terminated += 1;
      },
    };

    expect(
      initializeEngineWorker(
        () => worker,
        () => canvas,
        init,
      ),
    ).toBe(worker);
    expect(posts).toEqual([{ message: { ...init, canvas }, transfer: [canvas] }]);
    expect(terminated).toBe(0);
  });

  it("does not transfer or terminate when worker creation fails", () => {
    const failure = new Error("worker blocked");
    let transferred = 0;

    expect(() =>
      initializeEngineWorker(
        () => {
          throw failure;
        },
        () => {
          transferred += 1;
          return {} as OffscreenCanvas;
        },
        init,
      ),
    ).toThrow(failure);
    expect(transferred).toBe(0);
  });

  for (const stage of ["transfer", "post"] as const) {
    it(`terminates a worker when ${stage} fails`, () => {
      const failure = new Error(`${stage} failed`);
      let terminated = 0;
      let posted = 0;
      const worker: InitializableEngineWorker = {
        postMessage: () => {
          posted += 1;
          if (stage === "post") throw failure;
        },
        terminate: () => {
          terminated += 1;
        },
      };

      expect(() =>
        initializeEngineWorker(
          () => worker,
          () => {
            if (stage === "transfer") throw failure;
            return {} as OffscreenCanvas;
          },
          init,
        ),
      ).toThrow(failure);
      expect(terminated).toBe(1);
      expect(posted).toBe(stage === "post" ? 1 : 0);
    });
  }
});
