import { GameController } from "./game-controller.ts";

declare global {
  interface Window {
    __VOXELS_GAME__?: {
      controller: GameController;
      snapshot(): ReturnType<GameController["getDebugSnapshot"]>;
      snapshotResidentWorld(): ReturnType<GameController["snapshotResidentWorld"]>;
      requestPointerLock(): Promise<void>;
      teleport(x: number, y: number, z: number): void;
      setViewDistance(chunks: number): void;
      forceResidencyUpdate(): ReturnType<GameController["forceResidencyUpdate"]>;
      teleportAndSettle(
        x: number,
        y: number,
        z: number,
        options?: { radiusChunks?: number },
      ): ReturnType<GameController["teleportAndSettle"]>;
      benchmarkChunkCrossing(
        iterations: number,
        chunkDelta?: number,
      ): ReturnType<GameController["benchmarkChunkCrossing"]>;
    };
  }
}

interface GameRuntime {
  controller: GameController;
  ready: Promise<void>;
  dispose(): void;
}

const runtime = mountGame();
reportAsyncFailure(runtime.ready);

if (import.meta.hot) {
  import.meta.hot.accept();
  import.meta.hot.dispose(() => {
    runtime.dispose();
  });
}

function mountGame(): GameRuntime {
  const appRoot = document.querySelector<HTMLElement>("[data-app='game']");
  if (!appRoot) {
    throw new Error("Game root not found");
  }

  const canvas = appRoot.querySelector<HTMLCanvasElement>("[data-role='viewport']");
  const telemetryElement = appRoot.querySelector<HTMLElement>("[data-role='telemetry']");
  const captureButton = appRoot.querySelector<HTMLButtonElement>("[data-role='capture']");

  if (!canvas || !telemetryElement || !captureButton) {
    throw new Error("Game UI is incomplete");
  }

  const controller = new GameController(canvas);
  const handleCaptureClick = async () => {
    await controller.requestPointerLock();
  };

  controller.onHudUpdate = (snapshot) => {
    telemetryElement.innerHTML = [
      metric("Position", formatPosition(snapshot.position)),
      metric("Feet", formatPosition(snapshot.feetPosition)),
      metric("Player Chunk", snapshot.playerChunk.join(", ")),
      metric("Stream Anchor", snapshot.streamAnchorChunk.join(", ")),
      metric("Grounded", snapshot.grounded ? "Yes" : "No"),
      metric("Yaw", `${snapshot.yawDegrees.toFixed(1)}°`),
      metric("Pitch", `${snapshot.pitchDegrees.toFixed(1)}°`),
      metric("Resident Chunks", snapshot.chunkCount.toLocaleString()),
      metric("Dirty Resident", snapshot.streamDirtyResidentChunks.toLocaleString()),
      metric("Radius", `${snapshot.residencyRadiusChunks} chunks`),
      metric("Surface Y", snapshot.surfaceY.toLocaleString()),
      metric("Voxels", snapshot.solidVoxelCount.toLocaleString()),
      metric("Palette", snapshot.paletteCount.toLocaleString()),
      metric("Stream", `${snapshot.streamMs.toFixed(1)} ms`),
      metric("Generated", snapshot.streamGeneratedChunks.toLocaleString()),
      metric("Evicted", snapshot.streamEvictedChunks.toLocaleString()),
      metric("Empty Skipped", snapshot.streamEmptyChunksSkipped.toLocaleString()),
      metric("Empty Cache Hits", snapshot.streamCachedEmptyChunkHits.toLocaleString()),
      metric("Mesh", `${snapshot.meshMs.toFixed(1)} ms`),
      metric("New Meshes", snapshot.meshNewChunks.toLocaleString()),
      metric("Remeshes", snapshot.meshRemeshChunks.toLocaleString()),
      metric("Sync", `${snapshot.lastFrameSyncMs.toFixed(2)} ms`),
      metric("Upload", `${snapshot.lastFrameUploadMs.toFixed(2)} ms`),
      metric("Upload Chunks", snapshot.lastFrameUploadChunks.toLocaleString()),
      metric("Draw Calls", snapshot.drawCalls.toLocaleString()),
      metric("Triangles", snapshot.triangles.toLocaleString()),
      metric("Frame CPU", `${snapshot.lastFrameCpuMs.toFixed(2)} ms`),
      metric("Avg Frame CPU", `${snapshot.avgFrameCpuMs.toFixed(2)} ms`),
    ].join("");
    captureButton.hidden = snapshot.pointerLocked;
  };

  captureButton.addEventListener("click", handleCaptureClick);
  window.__VOXELS_GAME__ = {
    controller,
    snapshot: () => controller.getDebugSnapshot(),
    snapshotResidentWorld: () => controller.snapshotResidentWorld(),
    requestPointerLock: () => controller.requestPointerLock(),
    teleport: (x, y, z) => controller.teleport([x, y, z]),
    setViewDistance: (chunks) => controller.setResidencyRadiusChunks(chunks),
    forceResidencyUpdate: () => controller.forceResidencyUpdate(),
    teleportAndSettle: (x, y, z, options) => controller.teleportAndSettle([x, y, z], options),
    benchmarkChunkCrossing: (iterations, chunkDelta) => controller.benchmarkChunkCrossing(iterations, chunkDelta),
  };

  const ready = controller.init();

  return {
    controller,
    ready,
    dispose() {
      captureButton.removeEventListener("click", handleCaptureClick);
      controller.onHudUpdate = null;
      controller.dispose();
      delete window.__VOXELS_GAME__;
    },
  };
}

function reportAsyncFailure(promise: Promise<unknown>): void {
  promise.catch((error) => {
    queueMicrotask(() => {
      throw error;
    });
  });
}

function metric(label: string, value: string): string {
  return `<div class="game-metric"><span>${label}</span><strong>${value}</strong></div>`;
}

function formatPosition(position: [number, number, number]): string {
  return position.map((value) => value.toFixed(1)).join(", ");
}
