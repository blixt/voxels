import { GameController } from "./game-controller.ts";

declare global {
  interface Window {
    __VOXELS_GAME__?: {
      controller: GameController;
      snapshot(): ReturnType<GameController["getDebugSnapshot"]>;
      requestPointerLock(): Promise<void>;
      teleport(x: number, y: number, z: number): void;
    };
  }
}

const appRoot = document.querySelector<HTMLElement>("[data-app='game']");
if (!appRoot) {
  throw new Error("Game root not found");
}

const canvas = appRoot.querySelector<HTMLCanvasElement>("[data-role='viewport']");
const statusElement = appRoot.querySelector<HTMLElement>("[data-role='status']");
const telemetryElement = appRoot.querySelector<HTMLElement>("[data-role='telemetry']");
const captureButton = appRoot.querySelector<HTMLButtonElement>("[data-role='capture']");

if (!canvas || !statusElement || !telemetryElement || !captureButton) {
  throw new Error("Game UI is incomplete");
}

const controller = new GameController(canvas);
window.__VOXELS_GAME__ = {
  controller,
  snapshot: () => controller.getDebugSnapshot(),
  requestPointerLock: () => controller.requestPointerLock(),
  teleport: (x, y, z) => controller.teleport([x, y, z]),
};

controller.onHudUpdate = (snapshot) => {
  statusElement.textContent = snapshot.status;
  telemetryElement.innerHTML = [
    metric("Position", formatPosition(snapshot.position)),
    metric("Yaw", `${snapshot.yawDegrees.toFixed(1)}°`),
    metric("Pitch", `${snapshot.pitchDegrees.toFixed(1)}°`),
    metric("Chunks", snapshot.chunkCount.toLocaleString()),
    metric("Voxels", snapshot.solidVoxelCount.toLocaleString()),
    metric("Palette", snapshot.paletteCount.toLocaleString()),
    metric("Build", `${snapshot.buildMs.toFixed(1)} ms`),
    metric("Mesh", `${snapshot.meshMs.toFixed(1)} ms`),
    metric("Draw Calls", snapshot.drawCalls.toLocaleString()),
    metric("Triangles", snapshot.triangles.toLocaleString()),
    metric("Frame CPU", `${snapshot.avgFrameCpuMs.toFixed(2)} ms`),
  ].join("");
  captureButton.hidden = snapshot.pointerLocked;
};

captureButton.addEventListener("click", async () => {
  await controller.requestPointerLock();
});

await controller.init();

function metric(label: string, value: string): string {
  return `<div class="game-metric"><span>${label}</span><strong>${value}</strong></div>`;
}

function formatPosition(position: [number, number, number]): string {
  return position.map((value) => value.toFixed(1)).join(", ");
}
