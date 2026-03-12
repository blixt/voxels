import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { createServer } from "node:net";

export interface BenchmarkHarnessOptions {
  chromeBinary?: string;
  headless?: boolean;
  appPort?: number | null;
  viewportWidth?: number;
  viewportHeight?: number;
  skipBuild?: boolean;
  outputDir?: string | null;
}

export interface CommandResult {
  exitCode: number;
  elapsedMs: number;
  stdout: string;
  stderr: string;
}

export interface BrowserMemorySample {
  elapsedMs: number;
  jsHeapUsedSizeBytes: number | null;
  jsHeapTotalSizeBytes: number | null;
  runtimeHeapUsedBytes: number | null;
  runtimeHeapTotalBytes: number | null;
  taskDurationSeconds: number | null;
  scriptDurationSeconds: number | null;
  layoutDurationSeconds: number | null;
  recalcStyleDurationSeconds: number | null;
  documents: number | null;
  nodes: number | null;
  generationWorkerCount: number | null;
  residentChunks: number | null;
  pendingChunks: number | null;
  dirtyResidentChunks: number | null;
  farFieldPendingBands: number | null;
  solidVoxelCount: number | null;
}

export interface BenchmarkIterationRun {
  warmup: boolean;
  iteration: number;
  globalIndex: number;
}

export interface BrowserBenchmarkStatus<TResult> {
  done: boolean;
  result: TResult | null;
}

export interface BrowserBenchmarkScenario<TResult> {
  id: string;
  description: string;
  warmupIterations: number;
  measuredIterations: number;
  timeoutMs: number;
  sampleIntervalMs: number;
  benchmarkStartsDuringPrepare?: boolean;
  prepareIteration(session: BrowserGameSession, run: BenchmarkIterationRun): Promise<void>;
  readIteration(session: BrowserGameSession, run: BenchmarkIterationRun): Promise<BrowserBenchmarkStatus<TResult>>;
}

export interface BrowserBenchmarkIterationResult<TResult> {
  scenarioId: string;
  warmup: boolean;
  iteration: number;
  globalIndex: number;
  setupElapsedMs: number;
  benchmarkElapsedMs: number;
  pollSamples: BrowserMemorySample[];
  result: TResult;
}

export interface BrowserMemorySummary {
  sampleCount: number;
  firstElapsedMs: number | null;
  lastElapsedMs: number | null;
  peakJsHeapUsedSizeBytes: number | null;
  peakJsHeapTotalSizeBytes: number | null;
  peakRuntimeHeapUsedBytes: number | null;
  peakRuntimeHeapTotalBytes: number | null;
  peakGenerationWorkerCount: number | null;
  peakResidentChunks: number | null;
  peakPendingChunks: number | null;
  peakDirtyResidentChunks: number | null;
  peakFarFieldPendingBands: number | null;
  peakSolidVoxelCount: number | null;
  deltaTaskDurationMs: number | null;
  deltaScriptDurationMs: number | null;
  deltaLayoutDurationMs: number | null;
  deltaRecalcStyleDurationMs: number | null;
}

export interface BenchmarkArtifactPaths {
  reportJsonPath: string;
  iterationCsvPath: string;
  samplesCsvPath: string | null;
  memoryCsvPath: string;
}

export interface AsyncWindowBenchmarkState<TResult> {
  status: "running" | "completed" | "failed";
  startedAtMs: number;
  result: TResult | null;
  error: string | null;
}

export interface BrowserGameSession {
  readonly appUrl: string;
  readonly outputDir: string;
  readonly build: CommandResult | null;
  navigateToGame(options?: {
    clearStorage?: boolean;
    query?: Record<string, string | number | boolean>;
  }): Promise<void>;
  waitForWindowGameApi(timeoutMs: number): Promise<void>;
  waitForBootstrapBenchmarkComplete(timeoutMs: number): Promise<void>;
  waitForGameReady(timeoutMs: number): Promise<void>;
  evaluate<T>(expression: string): Promise<T>;
  startAsyncWindowBenchmark(startExpression: string): Promise<string>;
  getAsyncWindowBenchmarkState<TResult>(token: string): Promise<AsyncWindowBenchmarkState<TResult> | null>;
  captureMemorySample(elapsedMs: number): Promise<BrowserMemorySample>;
}

interface DevToolsVersionResponse {
  webSocketDebuggerUrl: string;
}

interface DevToolsTargetResponse {
  id: string;
  webSocketDebuggerUrl: string;
}

interface CdpMessage {
  id?: number;
  method?: string;
  params?: Record<string, unknown>;
  result?: Record<string, unknown>;
  error?: {
    code: number;
    message: string;
  };
}

export interface CdpConnection {
  close(): Promise<void>;
  send(method: string, params?: Record<string, unknown>): Promise<Record<string, unknown>>;
  waitForEvent(
    method: string,
    timeoutMs: number,
    predicate?: ((params: Record<string, unknown>) => boolean) | null,
  ): Promise<Record<string, unknown>>;
  evaluate<T>(expression: string): Promise<T>;
}

export async function createBenchmarkOutputDir(
  explicitOutputDir: string | null = null,
  prefix = "voxels-browser-bench",
): Promise<string> {
  if (explicitOutputDir) {
    await Bun.$`mkdir -p ${explicitOutputDir}`.quiet();
    return explicitOutputDir;
  }
  return await mkdtemp(join(tmpdir(), `${prefix}-`));
}

export async function withBrowserGameSession<T>(
  options: BenchmarkHarnessOptions,
  fn: (session: BrowserGameSession) => Promise<T>,
): Promise<T> {
  const outputDir = await createBenchmarkOutputDir(options.outputDir ?? null);
  const appPort = options.appPort ?? await findFreePort();
  const devToolsPort = await findFreePort();
  const appUrl = `http://127.0.0.1:${appPort}/`;
  const chromeBinary = options.chromeBinary ?? resolveChromeBinary();
  const headless = options.headless ?? true;
  const viewportWidth = options.viewportWidth ?? 1440;
  const viewportHeight = options.viewportHeight ?? 900;
  const skipBuild = options.skipBuild ?? false;

  let build: CommandResult | null = null;
  let serverProcess: Bun.Subprocess | null = null;
  let chromeProcess: Bun.Subprocess | null = null;
  let cdp: CdpConnection | null = null;
  let chromeProfileDir: string | null = null;

  try {
    if (!skipBuild) {
      build = runCommand(["bun", "run", "build"]);
      if (build.exitCode !== 0) {
        throw new Error(`Build failed:\n${build.stderr || build.stdout}`);
      }
    }

    serverProcess = Bun.spawn(["bun", "run", "start"], {
      cwd: process.cwd(),
      env: {
        ...process.env,
        NODE_ENV: "production",
        PORT: String(appPort),
      },
      stdout: "ignore",
      stderr: "pipe",
    });
    await waitForHttp(appUrl, 15_000);

    chromeProfileDir = await mkdtemp(join(tmpdir(), "voxels-browser-bench-chrome-"));
    chromeProcess = Bun.spawn(buildChromeCommand(chromeBinary, devToolsPort, chromeProfileDir, headless), {
      cwd: process.cwd(),
      stdout: "ignore",
      stderr: "ignore",
    });
    await waitForJsonEndpoint<DevToolsVersionResponse>(`http://127.0.0.1:${devToolsPort}/json/version`, 15_000);

    const target = await createDevToolsTarget(devToolsPort);
    cdp = await connectCdp(target.webSocketDebuggerUrl);
    await cdp.send("Page.enable");
    await cdp.send("Runtime.enable");
    await cdp.send("Performance.enable");
    await cdp.send("Emulation.setDeviceMetricsOverride", {
      width: viewportWidth,
      height: viewportHeight,
      deviceScaleFactor: 1,
      mobile: false,
    });

    const session = new BrowserGameSessionImpl(cdp, appUrl, outputDir, build);
    return await fn(session);
  } finally {
    await cdp?.close();
    chromeProcess?.kill();
    serverProcess?.kill();
    if (chromeProfileDir) {
      await rm(chromeProfileDir, { recursive: true, force: true });
    }
  }
}

export async function runBrowserBenchmarkScenario<TResult>(
  session: BrowserGameSession,
  scenario: BrowserBenchmarkScenario<TResult>,
): Promise<BrowserBenchmarkIterationResult<TResult>[]> {
  const results: BrowserBenchmarkIterationResult<TResult>[] = [];
  const totalIterations = scenario.warmupIterations + scenario.measuredIterations;
  for (let globalIndex = 0; globalIndex < totalIterations; globalIndex += 1) {
    const run: BenchmarkIterationRun = {
      warmup: globalIndex < scenario.warmupIterations,
      iteration: globalIndex < scenario.warmupIterations
        ? globalIndex + 1
        : globalIndex - scenario.warmupIterations + 1,
      globalIndex,
    };
    console.log(
      `[${scenario.id}] starting ${run.warmup ? "warmup" : "measured"} iteration ${run.iteration}`
      + ` (${globalIndex + 1}/${totalIterations})`,
    );
    const setupStartedAt = performance.now();
    const benchmarkStartsAt = scenario.benchmarkStartsDuringPrepare ? setupStartedAt : 0;
    await scenario.prepareIteration(session, run);
    const setupElapsedMs = performance.now() - setupStartedAt;
    console.log(
      `[${scenario.id}] prepared ${run.warmup ? "warmup" : "measured"} iteration ${run.iteration}`
      + ` in ${Number(setupElapsedMs.toFixed(3))} ms`,
    );
    const benchmarkStartedAt = scenario.benchmarkStartsDuringPrepare ? benchmarkStartsAt : performance.now();
    const pollSamples: BrowserMemorySample[] = [];
    let result: TResult | null = null;
    while (performance.now() - benchmarkStartedAt <= scenario.timeoutMs) {
      const status = await scenario.readIteration(session, run);
      pollSamples.push(await session.captureMemorySample(performance.now() - benchmarkStartedAt));
      if (status.done) {
        result = status.result;
        break;
      }
      await Bun.sleep(scenario.sampleIntervalMs);
    }
    if (result === null) {
      throw new Error(`Benchmark scenario "${scenario.id}" timed out after ${scenario.timeoutMs} ms`);
    }
    results.push({
      scenarioId: scenario.id,
      warmup: run.warmup,
      iteration: run.iteration,
      globalIndex: run.globalIndex,
      setupElapsedMs: Number(setupElapsedMs.toFixed(3)),
      benchmarkElapsedMs: Number((performance.now() - benchmarkStartedAt).toFixed(3)),
      pollSamples,
      result,
    });
    console.log(
      `[${scenario.id}] completed ${run.warmup ? "warmup" : "measured"} iteration ${run.iteration}`
      + ` in ${Number((performance.now() - benchmarkStartedAt).toFixed(3))} ms`,
    );
  }
  return results;
}

export function summarizeMemorySamples(samples: readonly BrowserMemorySample[]): BrowserMemorySummary {
  return {
    sampleCount: samples.length,
    firstElapsedMs: samples[0]?.elapsedMs ?? null,
    lastElapsedMs: samples[samples.length - 1]?.elapsedMs ?? null,
    peakJsHeapUsedSizeBytes: maxNullable(samples.map((sample) => sample.jsHeapUsedSizeBytes)),
    peakJsHeapTotalSizeBytes: maxNullable(samples.map((sample) => sample.jsHeapTotalSizeBytes)),
    peakRuntimeHeapUsedBytes: maxNullable(samples.map((sample) => sample.runtimeHeapUsedBytes)),
    peakRuntimeHeapTotalBytes: maxNullable(samples.map((sample) => sample.runtimeHeapTotalBytes)),
    peakGenerationWorkerCount: maxNullable(samples.map((sample) => sample.generationWorkerCount)),
    peakResidentChunks: maxNullable(samples.map((sample) => sample.residentChunks)),
    peakPendingChunks: maxNullable(samples.map((sample) => sample.pendingChunks)),
    peakDirtyResidentChunks: maxNullable(samples.map((sample) => sample.dirtyResidentChunks)),
    peakFarFieldPendingBands: maxNullable(samples.map((sample) => sample.farFieldPendingBands)),
    peakSolidVoxelCount: maxNullable(samples.map((sample) => sample.solidVoxelCount)),
    deltaTaskDurationMs: deltaSecondsToMs(samples, "taskDurationSeconds"),
    deltaScriptDurationMs: deltaSecondsToMs(samples, "scriptDurationSeconds"),
    deltaLayoutDurationMs: deltaSecondsToMs(samples, "layoutDurationSeconds"),
    deltaRecalcStyleDurationMs: deltaSecondsToMs(samples, "recalcStyleDurationSeconds"),
  };
}

export async function writeBenchmarkArtifacts<TResult>(
  outputDir: string,
  scenarioId: string,
  iterations: readonly BrowserBenchmarkIterationResult<TResult>[],
  options: {
    buildIterationRow(iteration: BrowserBenchmarkIterationResult<TResult>, memory: BrowserMemorySummary): Record<string, string | number | boolean | null | undefined>;
    buildSampleRows?: (iteration: BrowserBenchmarkIterationResult<TResult>) => readonly Record<string, string | number | boolean | null | undefined>[];
    buildReport?: (iterationRows: readonly Record<string, string | number | boolean | null | undefined>[]) => Record<string, unknown>;
  },
): Promise<BenchmarkArtifactPaths> {
  const iterationRows: Record<string, string | number | boolean | null | undefined>[] = [];
  const sampleRows: Record<string, string | number | boolean | null | undefined>[] = [];
  const memoryRows: Record<string, string | number | boolean | null | undefined>[] = [];

  for (const iteration of iterations) {
    const memory = summarizeMemorySamples(iteration.pollSamples);
    iterationRows.push(options.buildIterationRow(iteration, memory));
    if (options.buildSampleRows) {
      sampleRows.push(...options.buildSampleRows(iteration));
    }
    memoryRows.push(...iteration.pollSamples.map((sample) => ({
      scenarioId: iteration.scenarioId,
      warmup: iteration.warmup,
      iteration: iteration.iteration,
      globalIndex: iteration.globalIndex,
      ...sample,
    })));
  }

  const iterationCsvPath = join(outputDir, `${scenarioId}-iterations.csv`);
  const samplesCsvPath = sampleRows.length > 0 ? join(outputDir, `${scenarioId}-samples.csv`) : null;
  const memoryCsvPath = join(outputDir, `${scenarioId}-memory.csv`);
  const reportJsonPath = join(outputDir, `${scenarioId}-report.json`);

  await writeCsv(iterationCsvPath, iterationRows);
  if (samplesCsvPath) {
    await writeCsv(samplesCsvPath, sampleRows);
  }
  await writeCsv(memoryCsvPath, memoryRows);
  await Bun.write(reportJsonPath, `${JSON.stringify({
    scenarioId,
    generatedAt: new Date().toISOString(),
    iterations: iterationRows,
    report: options.buildReport?.(iterationRows) ?? null,
    csvPaths: {
      iterationCsvPath,
      samplesCsvPath,
      memoryCsvPath,
    },
  }, null, 2)}\n`);

  return {
    reportJsonPath,
    iterationCsvPath,
    samplesCsvPath,
    memoryCsvPath,
  };
}

export async function writeCsv(
  filePath: string,
  rows: readonly Record<string, string | number | boolean | null | undefined>[],
): Promise<void> {
  const headers = collectCsvHeaders(rows);
  const lines = [headers.join(",")];
  for (const row of rows) {
    lines.push(headers.map((header) => encodeCsvCell(row[header])).join(","));
  }
  await Bun.write(filePath, `${lines.join("\n")}\n`);
}

export function timestampForFile(date: Date): string {
  return date.toISOString().replace(/[:-]/g, "").replace(/\.\d{3}Z$/, "Z");
}

export function sanitizeFileStem(value: string): string {
  return value.replace(/[^A-Za-z0-9._-]+/g, "-").replace(/-+/g, "-").replace(/^-|-$/g, "");
}

export function readGitShortHead(): string | null {
  const result = Bun.spawnSync(["git", "rev-parse", "--short", "HEAD"], {
    cwd: process.cwd(),
    stdout: "pipe",
    stderr: "pipe",
  });
  if (result.exitCode !== 0) {
    return null;
  }
  return decodeBytes(result.stdout).trim() || null;
}

export function resolveChromeBinary(): string {
  const candidatePaths = [
    process.env.CHROME_BINARY,
    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    "/Applications/Google Chrome Beta.app/Contents/MacOS/Google Chrome Beta",
    "/Applications/Google Chrome Canary.app/Contents/MacOS/Google Chrome Canary",
    "/usr/bin/google-chrome",
    "/usr/bin/google-chrome-stable",
    "/usr/bin/chromium",
  ].filter((value): value is string => Boolean(value));
  const existing = candidatePaths.find((candidate) => Bun.file(candidate).size > -1);
  if (!existing) {
    throw new Error("Unable to resolve a Chrome binary. Pass --chrome-binary or set CHROME_BINARY.");
  }
  return existing;
}

function maxNullable(values: readonly (number | null)[]): number | null {
  let maxValue: number | null = null;
  for (const value of values) {
    if (value === null) {
      continue;
    }
    maxValue = maxValue === null ? value : Math.max(maxValue, value);
  }
  return maxValue;
}

function deltaSecondsToMs(
  samples: readonly BrowserMemorySample[],
  key: "taskDurationSeconds" | "scriptDurationSeconds" | "layoutDurationSeconds" | "recalcStyleDurationSeconds",
): number | null {
  const first = firstNumber(samples.map((sample) => sample[key]));
  const last = lastNumber(samples.map((sample) => sample[key]));
  if (first === null || last === null) {
    return null;
  }
  return Number(((last - first) * 1000).toFixed(3));
}

function firstNumber(values: readonly (number | null)[]): number | null {
  for (const value of values) {
    if (value !== null && value !== undefined) {
      return value;
    }
  }
  return null;
}

function lastNumber(values: readonly (number | null)[]): number | null {
  for (let index = values.length - 1; index >= 0; index -= 1) {
    const value = values[index];
    if (value !== null && value !== undefined) {
      return value;
    }
  }
  return null;
}

function collectCsvHeaders(
  rows: readonly Record<string, string | number | boolean | null | undefined>[],
): string[] {
  const headers = new Set<string>();
  for (const row of rows) {
    for (const key of Object.keys(row)) {
      headers.add(key);
    }
  }
  return [...headers];
}

function encodeCsvCell(value: string | number | boolean | null | undefined): string {
  if (value === null || value === undefined) {
    return "";
  }
  const normalized = typeof value === "string" ? value : String(value);
  if (!/[",\n]/.test(normalized)) {
    return normalized;
  }
  return `"${normalized.replace(/"/g, "\"\"")}"`;
}

function runCommand(command: string[]): CommandResult {
  const startedAt = performance.now();
  const subprocess = Bun.spawnSync(command, {
    cwd: process.cwd(),
    stdout: "pipe",
    stderr: "pipe",
    env: process.env,
  });
  const elapsedMs = performance.now() - startedAt;
  return {
    exitCode: subprocess.exitCode,
    elapsedMs: Number(elapsedMs.toFixed(2)),
    stdout: decodeBytes(subprocess.stdout),
    stderr: decodeBytes(subprocess.stderr),
  };
}

function buildChromeCommand(
  chromeBinary: string,
  devToolsPort: number,
  profileDir: string,
  headless: boolean,
): string[] {
  const command = [
    chromeBinary,
    `--remote-debugging-port=${devToolsPort}`,
    `--user-data-dir=${profileDir}`,
    "--no-first-run",
    "--no-default-browser-check",
    "--disable-background-networking",
    "--disable-component-update",
    "--disable-sync",
    "--disable-features=DialMediaRouteProvider",
    "--enable-unsafe-webgpu",
    "--enable-features=Vulkan,UseSkiaRenderer",
    "about:blank",
  ];
  if (headless) {
    command.splice(1, 0, "--headless=new");
  }
  return command;
}

async function waitForHttp(url: string, timeoutMs: number): Promise<void> {
  await pollUntil(async () => {
    const response = await fetch(url);
    if (!response.ok) {
      throw new Error(`Expected HTTP 200 from ${url}, received ${response.status}`);
    }
  }, timeoutMs);
}

async function waitForJsonEndpoint<T>(url: string, timeoutMs: number): Promise<T> {
  return await pollUntil(async () => {
    const response = await fetch(url);
    if (!response.ok) {
      throw new Error(`Expected HTTP 200 from ${url}, received ${response.status}`);
    }
    return await response.json() as T;
  }, timeoutMs);
}

async function createDevToolsTarget(devToolsPort: number): Promise<DevToolsTargetResponse> {
  return await pollUntil(async () => {
    const endpoint = `http://127.0.0.1:${devToolsPort}/json/new?about:blank`;
    const response = await fetch(endpoint, { method: "PUT" });
    if (response.ok) {
      return await response.json() as DevToolsTargetResponse;
    }
    const fallback = await fetch(endpoint);
    if (!fallback.ok) {
      throw new Error(`Failed to create DevTools target: ${response.status} / ${fallback.status}`);
    }
    return await fallback.json() as DevToolsTargetResponse;
  }, 15_000);
}

async function pollUntil<T>(
  attempt: () => Promise<T>,
  timeoutMs: number,
  intervalMs = 100,
): Promise<T> {
  const deadline = Date.now() + timeoutMs;
  let lastError: unknown = null;
  while (Date.now() < deadline) {
    try {
      return await attempt();
    } catch (error) {
      lastError = error;
      await Bun.sleep(intervalMs);
    }
  }
  throw lastError instanceof Error ? lastError : new Error("Timed out");
}

async function findFreePort(): Promise<number> {
  return await new Promise<number>((resolve, reject) => {
    const server = createServer();
    server.listen(0, "127.0.0.1");
    server.on("listening", () => {
      const address = server.address();
      if (!address || typeof address === "string") {
        reject(new Error("Unable to determine free port"));
        return;
      }
      const port = address.port;
      server.close((error) => {
        if (error) {
          reject(error);
          return;
        }
        resolve(port);
      });
    });
    server.on("error", reject);
  });
}

function decodeBytes(bytes: Uint8Array): string {
  return new TextDecoder().decode(bytes);
}

async function connectCdp(url: string): Promise<CdpConnection> {
  return await CdpClient.connect(url);
}

class BrowserGameSessionImpl implements BrowserGameSession {
  constructor(
    private readonly cdp: CdpConnection,
    readonly appUrl: string,
    readonly outputDir: string,
    readonly build: CommandResult | null,
  ) {}

  async navigateToGame(options: {
    clearStorage?: boolean;
    query?: Record<string, string | number | boolean>;
  } = {}): Promise<void> {
    if (options.clearStorage) {
      await this.cdp.send("Storage.clearDataForOrigin", {
        origin: new URL(this.appUrl).origin,
        storageTypes: "indexeddb,local_storage,cache_storage,service_workers,websql,filesystem,shader_cache",
      });
    }
    const query = options.query ? new URLSearchParams(
      Object.entries(options.query).map(([key, value]) => [key, String(value)]),
    ).toString() : "";
    const url = query ? `${this.appUrl}?${query}` : this.appUrl;
    const loadEvent = this.cdp.waitForEvent("Page.loadEventFired", 30_000);
    await this.cdp.send("Page.navigate", { url });
    await loadEvent;
  }

  async waitForWindowGameApi(timeoutMs: number): Promise<void> {
    await pollUntil(async () => {
      const ready = await this.evaluate<boolean>(`
        Boolean(
          window.__VOXELS_GAME__
          && typeof window.__VOXELS_GAME__.snapshot === "function"
          && typeof window.__VOXELS_GAME__.getBootstrapBenchmark === "function"
        )
      `);
      if (!ready) {
        throw new Error("window.__VOXELS_GAME__ is not ready");
      }
    }, timeoutMs);
  }

  async waitForBootstrapBenchmarkComplete(timeoutMs: number): Promise<void> {
    await this.waitForWindowGameApi(timeoutMs);
    await pollUntil(async () => {
      const ready = await this.evaluate<boolean>(`
        (() => {
          const game = window.__VOXELS_GAME__;
          if (!game) {
            return false;
          }
          const benchmark = game.getBootstrapBenchmark();
          const snapshot = game.snapshot();
          return (benchmark.samples.length > 0 || snapshot.chunkCount > 0)
            && snapshot.chunkCount > 0
            && snapshot.streamPendingChunks === 0
            && snapshot.streamDirtyResidentChunks === 0
            && snapshot.farFieldPendingBands === 0;
        })()
      `);
      if (!ready) {
        throw new Error("Bootstrap benchmark is not complete yet");
      }
    }, timeoutMs, 50);
  }

  async waitForGameReady(timeoutMs: number): Promise<void> {
    await this.waitForWindowGameApi(timeoutMs);
    await pollUntil(async () => {
      const snapshot = await this.evaluate<Record<string, unknown>>("window.__VOXELS_GAME__.snapshot()");
      const chunkCount = typeof snapshot.chunkCount === "number" ? snapshot.chunkCount : 0;
      if (chunkCount <= 0) {
        throw new Error("Game snapshot did not report resident chunks yet");
      }
    }, timeoutMs, 50);
  }

  async evaluate<T>(expression: string): Promise<T> {
    return await this.cdp.evaluate<T>(expression);
  }

  async startAsyncWindowBenchmark(startExpression: string): Promise<string> {
    const token = crypto.randomUUID();
    await this.evaluate(`
      (() => {
        window.__VOXELS_BENCH_RUNS__ ??= {};
        const token = ${JSON.stringify(token)};
        window.__VOXELS_BENCH_RUNS__[token] = {
          status: "running",
          startedAtMs: performance.now(),
          result: null,
          error: null,
        };
        Promise.resolve()
          .then(() => ${startExpression})
          .then(
            (result) => {
              window.__VOXELS_BENCH_RUNS__[token] = {
                status: "completed",
                startedAtMs: window.__VOXELS_BENCH_RUNS__[token].startedAtMs,
                result,
                error: null,
              };
            },
            (error) => {
              window.__VOXELS_BENCH_RUNS__[token] = {
                status: "failed",
                startedAtMs: window.__VOXELS_BENCH_RUNS__[token].startedAtMs,
                result: null,
                error: String(error && (error.stack || error.message || error)),
              };
            },
          );
        return token;
      })()
    `);
    return token;
  }

  async getAsyncWindowBenchmarkState<TResult>(token: string): Promise<AsyncWindowBenchmarkState<TResult> | null> {
    return await this.evaluate<AsyncWindowBenchmarkState<TResult> | null>(`
      (() => {
        const run = window.__VOXELS_BENCH_RUNS__?.[${JSON.stringify(token)}] ?? null;
        return run;
      })()
    `);
  }

  async captureMemorySample(elapsedMs: number): Promise<BrowserMemorySample> {
    const metricsResponse = await this.cdp.send("Performance.getMetrics");
    const runtimeHeap = await this.cdp.send("Runtime.getHeapUsage");
    const snapshot = await this.evaluate<Record<string, unknown> | null>(`
      (() => {
        const game = window.__VOXELS_GAME__;
        if (!game) {
          return null;
        }
        const snapshot = game.snapshot();
        return {
          chunkCount: snapshot.chunkCount,
          generationWorkerCount: snapshot.generationWorkerCount,
          pendingChunks: snapshot.streamPendingChunks,
          dirtyResidentChunks: snapshot.streamDirtyResidentChunks,
          farFieldPendingBands: snapshot.farFieldPendingBands,
          solidVoxelCount: snapshot.solidVoxelCount,
        };
      })()
    `);
    const metricValues = new Map(
      ((metricsResponse.metrics as Array<Record<string, unknown>> | undefined) ?? [])
        .map((metric) => [String(metric.name), typeof metric.value === "number" ? metric.value : null] as const),
    );
    return {
      elapsedMs: Number(elapsedMs.toFixed(3)),
      jsHeapUsedSizeBytes: metricValues.get("JSHeapUsedSize") ?? null,
      jsHeapTotalSizeBytes: metricValues.get("JSHeapTotalSize") ?? null,
      runtimeHeapUsedBytes: typeof runtimeHeap.usedSize === "number" ? runtimeHeap.usedSize : null,
      runtimeHeapTotalBytes: typeof runtimeHeap.totalSize === "number" ? runtimeHeap.totalSize : null,
      taskDurationSeconds: metricValues.get("TaskDuration") ?? null,
      scriptDurationSeconds: metricValues.get("ScriptDuration") ?? null,
      layoutDurationSeconds: metricValues.get("LayoutDuration") ?? null,
      recalcStyleDurationSeconds: metricValues.get("RecalcStyleDuration") ?? null,
      documents: metricValues.get("Documents") ?? null,
      nodes: metricValues.get("Nodes") ?? null,
      generationWorkerCount: typeof snapshot?.generationWorkerCount === "number" ? snapshot.generationWorkerCount : null,
      residentChunks: typeof snapshot?.chunkCount === "number" ? snapshot.chunkCount : null,
      pendingChunks: typeof snapshot?.pendingChunks === "number" ? snapshot.pendingChunks : null,
      dirtyResidentChunks: typeof snapshot?.dirtyResidentChunks === "number" ? snapshot.dirtyResidentChunks : null,
      farFieldPendingBands: typeof snapshot?.farFieldPendingBands === "number" ? snapshot.farFieldPendingBands : null,
      solidVoxelCount: typeof snapshot?.solidVoxelCount === "number" ? snapshot.solidVoxelCount : null,
    };
  }
}

class CdpClient implements CdpConnection {
  private readonly socket: WebSocket;
  private nextId = 1;
  private readonly pending = new Map<number, {
    resolve: (value: Record<string, unknown>) => void;
    reject: (reason?: unknown) => void;
  }>();
  private readonly waiters = new Map<string, Array<{
    resolve: (value: Record<string, unknown>) => void;
    reject: (reason?: unknown) => void;
    predicate: ((params: Record<string, unknown>) => boolean) | null;
    timer: Timer;
  }>>();

  private constructor(socket: WebSocket) {
    this.socket = socket;
    socket.onmessage = (event) => {
      const message = JSON.parse(String(event.data)) as CdpMessage;
      if (typeof message.id === "number") {
        const pending = this.pending.get(message.id);
        if (!pending) {
          return;
        }
        this.pending.delete(message.id);
        if (message.error) {
          pending.reject(new Error(message.error.message));
          return;
        }
        pending.resolve(message.result ?? {});
        return;
      }
      if (!message.method) {
        return;
      }
      const params = message.params ?? {};
      const waiters = this.waiters.get(message.method);
      if (!waiters || waiters.length === 0) {
        return;
      }
      for (let index = waiters.length - 1; index >= 0; index -= 1) {
        const waiter = waiters[index]!;
        if (waiter.predicate && !waiter.predicate(params)) {
          continue;
        }
        clearTimeout(waiter.timer);
        waiters.splice(index, 1);
        waiter.resolve(params);
      }
      if (waiters.length === 0) {
        this.waiters.delete(message.method);
      }
    };
  }

  static async connect(url: string): Promise<CdpClient> {
    const socket = new WebSocket(url);
    await new Promise<void>((resolve, reject) => {
      socket.onopen = () => resolve();
      socket.onerror = (event) => reject(event);
    });
    return new CdpClient(socket);
  }

  async close(): Promise<void> {
    for (const [id, pending] of this.pending) {
      this.pending.delete(id);
      pending.reject(new Error("CDP connection closed"));
    }
    for (const [, waiters] of this.waiters) {
      for (const waiter of waiters) {
        clearTimeout(waiter.timer);
        waiter.reject(new Error("CDP connection closed"));
      }
    }
    this.waiters.clear();
    this.socket.close();
  }

  send(method: string, params: Record<string, unknown> = {}): Promise<Record<string, unknown>> {
    const id = this.nextId;
    this.nextId += 1;
    return new Promise<Record<string, unknown>>((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
      this.socket.send(JSON.stringify({ id, method, params }));
    });
  }

  waitForEvent(
    method: string,
    timeoutMs: number,
    predicate: ((params: Record<string, unknown>) => boolean) | null = null,
  ): Promise<Record<string, unknown>> {
    return new Promise<Record<string, unknown>>((resolve, reject) => {
      const timer = setTimeout(() => {
        const waiters = this.waiters.get(method);
        if (!waiters) {
          reject(new Error(`Timed out waiting for ${method}`));
          return;
        }
        const index = waiters.findIndex((waiter) => waiter.resolve === resolve);
        if (index !== -1) {
          waiters.splice(index, 1);
        }
        reject(new Error(`Timed out waiting for ${method}`));
      }, timeoutMs);
      const waiters = this.waiters.get(method) ?? [];
      waiters.push({ resolve, reject, predicate, timer });
      this.waiters.set(method, waiters);
    });
  }

  async evaluate<T>(expression: string): Promise<T> {
    const response = await this.send("Runtime.evaluate", {
      expression,
      awaitPromise: true,
      returnByValue: true,
    });
    if ("exceptionDetails" in response) {
      throw new Error(`Runtime.evaluate failed: ${JSON.stringify(response.exceptionDetails)}`);
    }
    const result = response.result as Record<string, unknown> | undefined;
    return (result?.value ?? null) as T;
  }
}
