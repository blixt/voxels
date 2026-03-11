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

export function renderGamePage(assetVersion: string): string {
  return shell(
    "Voxels Game",
    "page-game",
    "game",
    `
      <section class="game-shell">
        <canvas class="game-viewport" data-role="viewport"></canvas>
        <div class="game-overlay">
          <header class="game-topbar">
            <div class="game-brand">
              <p class="eyebrow">Chrome 146 + WebGPU</p>
              <h1>Voxels</h1>
              <p class="game-subtitle">First-person runtime slice over the current engine baseline.</p>
            </div>
          </header>
          <div class="game-hud">
            <section class="game-panel game-panel-telemetry">
              <h2>Live Telemetry</h2>
              <div class="game-metrics" data-role="telemetry"></div>
            </section>
          </div>
          <div class="crosshair" aria-hidden="true"></div>
          <button class="capture-overlay" data-role="capture">Click To Enter The World</button>
        </div>
      </section>
    `,
    "game",
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
          <a href="/">Game</a>
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
          <button data-role="run-stress-button">Run Stress Suite</button>
          <button data-role="run-all-button">Run Full Suite</button>
          <div class="instructions">
            <h3>Automation</h3>
            <p>Use <code>/bench?auto=1&amp;scenario=terrain256&amp;iterations=2&amp;frames=60</code> to auto-run a scene.</p>
            <p>Use <code>/bench?auto=1&amp;suite=stress&amp;iterations=1&amp;frames=30</code> to run just the stress suite.</p>
            <p>The page also exposes <code>window.__VOXELS_BENCH__</code> for scripted control, including <code>runStress()</code> and <code>probeGeneration()</code>.</p>
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
