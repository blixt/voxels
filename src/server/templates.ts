function shell(
  title: string,
  bodyClass: string,
  appName: string,
  content: string,
  scriptName: string,
  assetVersion: string,
): string {
  return `<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>${title}</title>
    <link rel="stylesheet" href="/styles.css" />
  </head>
  <body class="${bodyClass}">
    <main class="shell" data-app="${appName}">
      ${content}
    </main>
    <script type="module" src="/build/${scriptName}.js?v=${assetVersion}"></script>
  </body>
</html>`;
}

export function renderPlaygroundPage(assetVersion: string): string {
  return shell(
    "Voxels Playground",
    "page-playground",
    "playground",
    `
      <section class="hero">
        <div>
          <p class="eyebrow">Chrome 146 + WebGPU</p>
          <h1>Voxel Playground</h1>
          <p class="lede">Chunked, editable voxel rendering from scratch with Bun, TypeScript, and a WebGPU pipeline tuned for lots of small voxels.</p>
        </div>
        <nav class="hero-links">
          <a href="/">Playground</a>
          <a href="/bench">Benchmark</a>
        </nav>
      </section>
      <section class="layout">
        <aside class="panel controls">
          <h2>Scene</h2>
          <label>
            <span>Preset</span>
            <select data-role="scene-select" name="scene"></select>
          </label>
          <label>
            <span>Import .vxsc / .vox</span>
            <input type="file" data-role="file-input" name="scene-file" accept=".vxsc,.vox" />
          </label>
          <label>
            <span>Import URL</span>
            <input type="url" data-role="import-url" name="scene-url" placeholder="https://example.com/model.vox" />
          </label>
          <button data-role="import-url-button">Load Remote Scene</button>
          <button data-role="export-button">Export .vxsc</button>
          <div class="instructions">
            <h3>Controls</h3>
            <p>Drag to orbit. Alt-drag or middle-drag to pan. Wheel to zoom.</p>
            <p>Click removes a voxel. Shift-click places one.</p>
          </div>
        </aside>
        <section class="viewport-frame">
          <canvas class="viewport"></canvas>
          <div class="status-bar" data-role="status">Initializing WebGPU</div>
        </section>
        <aside class="panel stats-panel">
          <h2>Live Stats</h2>
          <div class="stats-grid" data-role="stats"></div>
        </aside>
      </section>
    `,
    "playground",
    assetVersion,
  );
}

export function renderBenchPage(assetVersion: string): string {
  return shell(
    "Voxels Benchmark",
    "page-bench",
    "bench",
    `
      <section class="hero">
        <div>
          <p class="eyebrow">Repeatable verification</p>
          <h1>Benchmark Harness</h1>
          <p class="lede">Run repeatable scene suites against the current renderer and capture build, mesh, frame, and correctness data from tiny primitives through full 256^3 worlds.</p>
        </div>
        <nav class="hero-links">
          <a href="/">Playground</a>
          <a href="/bench">Benchmark</a>
        </nav>
      </section>
      <section class="layout">
        <aside class="panel controls">
          <h2>Run Suite</h2>
          <label>
            <span>Scene</span>
            <select data-role="scene-select" name="bench-scene"></select>
          </label>
          <label>
            <span>Iterations</span>
            <input type="number" min="1" step="1" value="3" data-role="iterations" name="bench-iterations" />
          </label>
          <label>
            <span>Frames / iteration</span>
            <input type="number" min="1" step="1" value="90" data-role="frames" name="bench-frames" />
          </label>
          <button data-role="run-button">Run Selected Scene</button>
          <button data-role="run-all-button">Run Full Suite</button>
          <div class="instructions">
            <h3>Automation</h3>
            <p>Use <code>/bench?auto=1&amp;scenario=terrain256&amp;iterations=2&amp;frames=60</code> to auto-run a scene.</p>
            <p>The page also exposes <code>window.__VOXELS_BENCH__</code> for scripted control.</p>
          </div>
        </aside>
        <section class="viewport-frame">
          <canvas class="viewport"></canvas>
          <div class="status-bar" data-role="status">Ready</div>
        </section>
        <aside class="panel results-panel">
          <h2>Results</h2>
          <div data-role="results" class="results"></div>
          <h3>Validation Preview</h3>
          <div data-role="preview" class="preview-panel">No validation preview available yet.</div>
        </aside>
      </section>
    `,
    "bench",
    assetVersion,
  );
}
