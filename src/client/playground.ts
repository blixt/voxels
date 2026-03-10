import { EngineController, getSceneOptions } from "./engine-controller.ts";

declare global {
  interface Window {
    __VOXELS_PLAYGROUND__?: EngineController;
  }
}

const appRoot = document.querySelector<HTMLElement>("[data-app='playground']");
if (!appRoot) {
  throw new Error("Playground root not found");
}

const canvas = appRoot.querySelector<HTMLCanvasElement>("canvas");
const statsElement = appRoot.querySelector<HTMLElement>("[data-role='stats']");
const statusElement = appRoot.querySelector<HTMLElement>("[data-role='status']");
const sceneSelect = appRoot.querySelector<HTMLSelectElement>("[data-role='scene-select']");
const fileInput = appRoot.querySelector<HTMLInputElement>("[data-role='file-input']");
const importUrlInput = appRoot.querySelector<HTMLInputElement>("[data-role='import-url']");
const importUrlButton = appRoot.querySelector<HTMLButtonElement>("[data-role='import-url-button']");
const exportButton = appRoot.querySelector<HTMLButtonElement>("[data-role='export-button']");

if (!canvas || !statsElement || !statusElement || !sceneSelect || !fileInput || !importUrlInput || !importUrlButton || !exportButton) {
  throw new Error("Playground UI is incomplete");
}
const statsRoot = statsElement;
const statusRoot = statusElement;
const presetSelect = sceneSelect;
const sceneFileInput = fileInput;
const importField = importUrlInput;
const importButton = importUrlButton;
const saveButton = exportButton;

for (const scene of getSceneOptions()) {
  const option = document.createElement("option");
  option.value = scene.id;
  option.textContent = scene.label;
  option.title = scene.description;
  sceneSelect.append(option);
}

const controller = new EngineController(canvas);
window.__VOXELS_PLAYGROUND__ = controller;
controller.onHudUpdate = (snapshot) => {
  statsRoot.innerHTML = [
    metric("Scene", snapshot.sceneName),
    metric("Solid Voxels", snapshot.solidVoxelCount.toLocaleString()),
    metric("Chunks", snapshot.chunkCount.toLocaleString()),
    metric("Palette", snapshot.paletteCount.toString()),
    metric("Build", `${snapshot.buildMs.toFixed(1)} ms`),
    metric("Mesh", `${snapshot.meshMs.toFixed(1)} ms`),
    metric("Draw Calls", snapshot.drawCalls.toString()),
    metric("Triangles", snapshot.triangles.toLocaleString()),
    metric("Frame CPU", `${snapshot.avgFrameCpuMs.toFixed(2)} ms`),
  ].join("");
  statusRoot.textContent = snapshot.status;
};

await controller.init("terrain256");

presetSelect.addEventListener("change", () => {
  controller.loadScene(presetSelect.value);
});

sceneFileInput.addEventListener("change", async () => {
  const file = sceneFileInput.files?.[0];
  if (!file) {
    return;
  }
  await controller.importSceneFile(file);
});

importButton.addEventListener("click", async () => {
  const value = importField.value.trim();
  if (!value) {
    return;
  }
  statusRoot.textContent = "Fetching external scene";
  const response = await fetch(value);
  if (!response.ok) {
    throw new Error(`Failed to fetch scene: ${response.status}`);
  }
  const blob = await response.blob();
  const file = new File([blob], value.split("/").at(-1) ?? "remote-scene", { type: blob.type });
  await controller.importSceneFile(file);
});

saveButton.addEventListener("click", () => {
  const blob = controller.exportScene();
  const url = URL.createObjectURL(blob);
  const anchor = document.createElement("a");
  anchor.href = url;
  anchor.download = `${controller.sceneName}.vxsc`;
  anchor.click();
  URL.revokeObjectURL(url);
});

function metric(label: string, value: string): string {
  return `<div class="metric"><span>${label}</span><strong>${value}</strong></div>`;
}
