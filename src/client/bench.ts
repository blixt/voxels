import { EngineController, getSceneOptions, getStressSceneOptions } from "./engine-controller.ts";
import type { SceneBenchmarkSample } from "../engine/types.ts";

declare global {
  interface Window {
    __VOXELS_BENCH__?: {
      run: (sceneId: string, iterations: number, frameCount: number) => Promise<unknown>;
      runStress: (iterations: number, frameCount: number) => Promise<unknown>;
      runAll: (iterations: number, frameCount: number) => Promise<unknown>;
      getLastResults: () => unknown;
      getLastValidationArtifacts: () => unknown;
    };
    __VOXELS_BENCH_CONTROLLER__?: EngineController;
  }
}

const appRoot = document.querySelector<HTMLElement>("[data-app='bench']");
if (!appRoot) {
  throw new Error("Bench root not found");
}

const canvas = appRoot.querySelector<HTMLCanvasElement>("canvas");
const sceneSelect = appRoot.querySelector<HTMLSelectElement>("[data-role='scene-select']");
const iterationsInput = appRoot.querySelector<HTMLInputElement>("[data-role='iterations']");
const framesInput = appRoot.querySelector<HTMLInputElement>("[data-role='frames']");
const runButton = appRoot.querySelector<HTMLButtonElement>("[data-role='run-button']");
const runStressButton = appRoot.querySelector<HTMLButtonElement>("[data-role='run-stress-button']");
const runAllButton = appRoot.querySelector<HTMLButtonElement>("[data-role='run-all-button']");
const statusElement = appRoot.querySelector<HTMLElement>("[data-role='status']");
const resultsElement = appRoot.querySelector<HTMLElement>("[data-role='results']");
const previewElement = appRoot.querySelector<HTMLElement>("[data-role='preview']");

if (!canvas || !sceneSelect || !iterationsInput || !framesInput || !runButton || !runStressButton || !runAllButton || !statusElement || !resultsElement || !previewElement) {
  throw new Error("Bench UI is incomplete");
}
const sceneField = sceneSelect;
const iterationsField = iterationsInput;
const framesField = framesInput;
const runSceneButton = runButton;
const runStressSuiteButton = runStressButton;
const runSuiteButton = runAllButton;
const statusRoot = statusElement;
const resultsRoot = resultsElement;
const previewRoot = previewElement;
const stressSceneIds = getStressSceneOptions().map((scene) => scene.id);

for (const scene of getSceneOptions()) {
  const option = document.createElement("option");
  option.value = scene.id;
  option.textContent = scene.label;
  option.title = scene.description;
  sceneSelect.append(option);
}

const controller = new EngineController(canvas);
window.__VOXELS_BENCH_CONTROLLER__ = controller;
await controller.init("terrain256");

let lastResults: unknown = null;

async function runScenario(sceneId: string, iterations: number, frames: number): Promise<void> {
  statusRoot.textContent = `Running ${sceneId} for ${iterations} iteration(s)`;
  const results = await controller.runBenchmark(sceneId, iterations, frames);
  lastResults = results;
  renderResults(resultsRoot, results);
  renderPreview(previewRoot);
  const allCorrect = results.every((sample) => sample.correctnessPass);
  statusRoot.textContent = allCorrect ? `Finished ${sceneId}` : `Finished ${sceneId} with correctness failures`;
}

async function runAll(iterations: number, frames: number): Promise<void> {
  await runSceneSet(getSceneOptions().map((scene) => scene.id), iterations, frames, "full suite");
}

async function runStress(iterations: number, frames: number): Promise<void> {
  await runSceneSet(stressSceneIds, iterations, frames, "stress suite");
}

async function runSceneSet(sceneIds: string[], iterations: number, frames: number, label: string): Promise<void> {
  if (sceneIds.length === 0) {
    statusRoot.textContent = `No scenes available for ${label}`;
    resultsRoot.innerHTML = "";
    previewRoot.innerHTML = "<p>No validation preview available yet.</p>";
    lastResults = [];
    return;
  }
  const aggregate = [];
  statusRoot.textContent = `Running ${label}`;
  for (const sceneId of sceneIds) {
    const results = await controller.runBenchmark(sceneId, iterations, frames);
    aggregate.push(...results);
  }
  lastResults = aggregate;
  renderResults(resultsRoot, aggregate);
  renderPreview(previewRoot);
  const allCorrect = aggregate.every((sample) => sample.correctnessPass);
  statusRoot.textContent = allCorrect ? `Finished ${label}` : `Finished ${label} with correctness failures`;
}

runSceneButton.addEventListener("click", async () => {
  await runScenario(sceneField.value, readInt(iterationsField.value, 3), readInt(framesField.value, 90));
});

runStressSuiteButton.addEventListener("click", async () => {
  await runStress(readInt(iterationsField.value, 3), readInt(framesField.value, 90));
});

runSuiteButton.addEventListener("click", async () => {
  await runAll(readInt(iterationsField.value, 3), readInt(framesField.value, 90));
});

window.__VOXELS_BENCH__ = {
  run: async (sceneId, iterations, frameCount) => {
    await runScenario(sceneId, iterations, frameCount);
    return lastResults;
  },
  runStress: async (iterations, frameCount) => {
    await runStress(iterations, frameCount);
    return lastResults;
  },
  runAll: async (iterations, frameCount) => {
    await runAll(iterations, frameCount);
    return lastResults;
  },
  getLastResults: () => lastResults,
  getLastValidationArtifacts: () => controller.getLastValidationArtifacts(),
};

const params = new URLSearchParams(window.location.search);
if (params.get("auto") === "1") {
  const scenario = params.get("scenario");
  const suite = params.get("suite");
  const iterations = readInt(params.get("iterations"), 2);
  const frames = readInt(params.get("frames"), 60);
  if (scenario) {
    void runScenario(scenario, iterations, frames);
  } else if (suite === "stress") {
    void runStress(iterations, frames);
  } else {
    void runAll(iterations, frames);
  }
}

function renderResults(target: HTMLElement, results: SceneBenchmarkSample[]): void {
  const rows = results.map((sample) => {
    const correctness = sample.correctnessPass ? "pass" : "fail";
    const visual = formatVisualStatus(sample.visualPass);
    return `<tr>
      <td>${sample.sceneName}</td>
      <td>${sample.sceneKind}</td>
      <td>${sample.iteration}</td>
      <td>${Number(sample.buildMs).toFixed(1)}</td>
      <td>${Number(sample.meshMs).toFixed(1)}</td>
      <td>${Number(sample.firstFrameCpuMs).toFixed(2)}</td>
      <td>${formatMetric(sample.avgWarmFrameCpuMs, 2)}</td>
      <td>${formatMetric(sample.firstFrameGpuMs, 2)}</td>
      <td>${formatMetric(sample.avgWarmFrameGpuMs, 2)}</td>
      <td>${Number(sample.firstFrameSyncMs).toFixed(2)}</td>
      <td>${Number(sample.firstFrameUploadMs).toFixed(2)}</td>
      <td>${Number(sample.firstFrameEncodeMs).toFixed(2)}</td>
      <td>${Number(sample.firstFrameUploadChunks).toLocaleString()}</td>
      <td>${formatBytes(sample.firstFrameUploadBytes)}</td>
      <td>${formatMetric(sample.meanAbsoluteError, 2)}</td>
      <td>${formatPercent(sample.coverageMismatchRatio)}</td>
      <td>${Number(sample.drawCalls).toLocaleString()}</td>
      <td>${Number(sample.triangles).toLocaleString()}</td>
      <td>${Number(sample.solidVoxelCount).toLocaleString()}</td>
      <td class="${visual.className}">${visual.label}</td>
      <td class="${correctness}">${correctness}</td>
      <td><code>${sample.checksum}</code></td>
    </tr>`;
  }).join("");

  target.innerHTML = `
    <table class="results-table">
      <thead>
        <tr>
          <th>Scene</th>
          <th>Kind</th>
          <th>Iter</th>
          <th>Build</th>
          <th>Mesh</th>
          <th>1st CPU</th>
          <th>Warm CPU</th>
          <th>1st GPU</th>
          <th>Warm GPU</th>
          <th>Sync</th>
          <th>Upload</th>
          <th>Encode</th>
          <th>Up Chunks</th>
          <th>Up Bytes</th>
          <th>MAE</th>
          <th>Mask Δ</th>
          <th>Draws</th>
          <th>Triangles</th>
          <th>Solid Voxels</th>
          <th>Visual</th>
          <th>Correct</th>
          <th>Checksum</th>
        </tr>
      </thead>
      <tbody>${rows}</tbody>
    </table>
  `;
}

function renderPreview(target: HTMLElement): void {
  const artifacts = controller.getLastValidationArtifacts();
  if (!artifacts) {
    target.innerHTML = "<p>No validation preview available yet.</p>";
    return;
  }
  target.innerHTML = `
    <div class="preview-grid">
      <figure>
        <img src="${artifacts.actualDataUrl}" alt="Actual render" />
        <figcaption>Actual</figcaption>
      </figure>
      <figure>
        <img src="${artifacts.referenceDataUrl}" alt="Reference render" />
        <figcaption>Reference</figcaption>
      </figure>
      <figure>
        <img src="${artifacts.diffDataUrl}" alt="Diff heatmap" />
        <figcaption>Diff</figcaption>
      </figure>
    </div>
  `;
}

function formatMetric(value: number | null, digits: number): string {
  return value === null ? "n/a" : Number(value).toFixed(digits);
}

function formatPercent(value: number | null): string {
  return value === null ? "n/a" : `${(value * 100).toFixed(2)}%`;
}

function formatBytes(value: number): string {
  if (value === 0) {
    return "0";
  }
  return `${(value / (1024 * 1024)).toFixed(2)} MB`;
}

function formatVisualStatus(value: boolean | null): { className: string; label: string } {
  if (value === null) {
    return { className: "", label: "n/a" };
  }
  return value
    ? { className: "pass", label: "pass" }
    : { className: "fail", label: "fail" };
}

function readInt(value: string | null, fallback: number): number {
  const parsed = Number.parseInt(value ?? "", 10);
  return Number.isFinite(parsed) && parsed > 0 ? parsed : fallback;
}
