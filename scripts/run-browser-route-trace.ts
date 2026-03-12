import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { createServer } from "node:net";

export {};

interface CliOptions {
  label: string | null;
  outputDir: string;
  chromeBinary: string;
  headless: boolean;
  appPort: number | null;
  viewportWidth: number;
  viewportHeight: number;
  skipBuild: boolean;
  durationSeconds: number;
  settleSeconds: number;
  sampleHz: number;
  speedMetersPerSecond: number;
  seamProbeStrideFrames: number;
  captureStrideFrames: number;
  referenceDiffStrideFrames: number;
  referenceDiffLimit: number;
}

interface CommandResult {
  exitCode: number;
  elapsedMs: number;
  stdout: string;
  stderr: string;
}

interface BrowserRouteTraceReport {
  generatedAt: string;
  label: string | null;
  commit: string | null;
  appUrl: string;
  chromeBinary: string;
  tracePath: string;
  benchmarkSamplesPath: string;
  routeOptions: Record<string, number>;
  build: CommandResult | null;
  benchmark: Record<string, unknown>;
  discovery: Record<string, unknown>;
  traceAnalysis: Record<string, unknown>;
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

interface CdpConnection {
  close(): Promise<void>;
  send(method: string, params?: Record<string, unknown>): Promise<Record<string, unknown>>;
  waitForEvent(
    method: string,
    timeoutMs: number,
    predicate?: ((params: Record<string, unknown>) => boolean) | null,
  ): Promise<Record<string, unknown>>;
  evaluate<T>(expression: string): Promise<T>;
}

const TRACE_CATEGORIES = [
  "devtools.timeline",
  "disabled-by-default-devtools.timeline",
  "v8.execute",
  "disabled-by-default-v8.cpu_profiler",
  "disabled-by-default-v8.cpu_profiler.hires",
];

const options = parseCli(Bun.argv);
const runStamp = timestampForFile(new Date());
const runName = `${runStamp}${options.label ? `-${sanitizeFileStem(options.label)}` : ""}`;
const outputDir = join(options.outputDir, runName);
const tracePath = join(outputDir, "trace.json");
const reportPath = join(outputDir, "report.json");
const benchmarkSamplesPath = join(outputDir, "benchmark-samples.json");
const appPort = options.appPort ?? await findFreePort();
const devToolsPort = await findFreePort();
const appUrl = `http://127.0.0.1:${appPort}/`;

await Bun.$`mkdir -p ${outputDir}`.quiet();

let build: CommandResult | null = null;
let serverProcess: Bun.Subprocess | null = null;
let chromeProcess: Bun.Subprocess | null = null;
let cdp: CdpConnection | null = null;
let chromeProfileDir: string | null = null;

try {
  if (!options.skipBuild) {
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

  chromeProfileDir = await mkdtemp(join(tmpdir(), "voxels-route-trace-"));
  chromeProcess = Bun.spawn(buildChromeCommand(
    options.chromeBinary,
    devToolsPort,
    chromeProfileDir,
    options.headless,
  ), {
    cwd: process.cwd(),
    stdout: "ignore",
    stderr: "ignore",
  });
  await waitForJsonEndpoint<DevToolsVersionResponse>(`http://127.0.0.1:${devToolsPort}/json/version`, 15_000);

  const target = await createDevToolsTarget(devToolsPort);
  cdp = await connectCdp(target.webSocketDebuggerUrl);
  await cdp.send("Page.enable");
  await cdp.send("Runtime.enable");
  await cdp.send("Emulation.setDeviceMetricsOverride", {
    width: options.viewportWidth,
    height: options.viewportHeight,
    deviceScaleFactor: 1,
    mobile: false,
  });
  const loadEvent = cdp.waitForEvent("Page.loadEventFired", 30_000);
  await cdp.send("Page.navigate", { url: appUrl });
  await loadEvent;

  await waitForGameReady(cdp, 30_000);
  const webgpuAvailable = await cdp.evaluate<boolean>("Boolean(navigator.gpu)");
  if (!webgpuAvailable) {
    throw new Error("Chrome page does not expose navigator.gpu");
  }

  await cdp.send("Tracing.start", {
    transferMode: "ReturnAsStream",
    categories: TRACE_CATEGORIES.join(","),
  });

  const routeOptions = {
    durationSeconds: options.durationSeconds,
    settleSeconds: options.settleSeconds,
    sampleHz: options.sampleHz,
    speedMetersPerSecond: options.speedMetersPerSecond,
    seamProbeStrideFrames: options.seamProbeStrideFrames,
    captureStrideFrames: options.captureStrideFrames,
    referenceDiffStrideFrames: options.referenceDiffStrideFrames,
    referenceDiffLimit: options.referenceDiffLimit,
  };
  const benchmarkResult = await cdp.evaluate<Record<string, unknown>>(`(async () => {
    const result = await window.__VOXELS_GAME__.benchmarkRouteExperience(${JSON.stringify(routeOptions)});
    return result;
  })()`);
  await Bun.write(benchmarkSamplesPath, `${JSON.stringify((benchmarkResult.samples ?? []) as unknown[], null, 2)}\n`);
  const benchmark = {
    seed: benchmarkResult.seed,
    radiusChunks: benchmarkResult.radiusChunks,
    durationSeconds: benchmarkResult.durationSeconds,
    settleSeconds: benchmarkResult.settleSeconds,
    totalDistanceMeters: benchmarkResult.totalDistanceMeters,
    sampleHz: benchmarkResult.sampleHz,
    speedMetersPerSecond: benchmarkResult.speedMetersPerSecond,
    sampleCount: Array.isArray(benchmarkResult.samples) ? benchmarkResult.samples.length : 0,
    summary: benchmarkResult.summary,
  };
  const discovery = await cdp.evaluate<Record<string, unknown>>(
    "window.__VOXELS_GAME__.getDiscoveryJournal()",
  );

  const tracingComplete = cdp.waitForEvent("Tracing.tracingComplete", 30_000);
  await cdp.send("Tracing.end");
  const tracingParams = await tracingComplete;
  const streamHandle = readStringField(tracingParams, "stream");
  if (!streamHandle) {
    throw new Error("Tracing.tracingComplete did not include a stream handle");
  }
  await writeProtocolStreamToFile(cdp, streamHandle, tracePath);

  const traceAnalysisResult = runCommand([
    "bun",
    "run",
    "analyze:trace",
    tracePath,
    `--url-prefix=${appUrl}`,
  ]);
  if (traceAnalysisResult.exitCode !== 0) {
    throw new Error(`Trace analysis failed:\n${traceAnalysisResult.stderr || traceAnalysisResult.stdout}`);
  }
  const traceAnalysis = JSON.parse(traceAnalysisResult.stdout) as Record<string, unknown>;

  const report: BrowserRouteTraceReport = {
    generatedAt: new Date().toISOString(),
    label: options.label,
    commit: readGitShortHead(),
    appUrl,
    chromeBinary: options.chromeBinary,
    tracePath,
    benchmarkSamplesPath,
    routeOptions,
    build,
    benchmark,
    discovery,
    traceAnalysis,
  };
  await Bun.write(reportPath, `${JSON.stringify(report, null, 2)}\n`);

  printReportSummary(reportPath, report);
} finally {
  await cdp?.close();
  chromeProcess?.kill();
  serverProcess?.kill();
  if (chromeProfileDir) {
    await rm(chromeProfileDir, { recursive: true, force: true });
  }
}

function parseCli(argv: readonly string[]): CliOptions {
  const args = argv.slice(2);
  return {
    label: readFlag(args, "--label"),
    outputDir: readFlag(args, "--output-dir") ?? "artifacts/browser-route-trace",
    chromeBinary: readFlag(args, "--chrome-binary") ?? resolveChromeBinary(),
    headless: readBooleanFlag(args, "--headless", true),
    appPort: readOptionalPositiveInt(readFlag(args, "--port")),
    viewportWidth: readPositiveInt(readFlag(args, "--width"), 1440),
    viewportHeight: readPositiveInt(readFlag(args, "--height"), 900),
    skipBuild: readBooleanFlag(args, "--skip-build", false),
    durationSeconds: readPositiveFloat(readFlag(args, "--duration"), 10),
    settleSeconds: readPositiveFloat(readFlag(args, "--settle"), 4),
    sampleHz: readPositiveInt(readFlag(args, "--sample-hz"), 60),
    speedMetersPerSecond: readPositiveFloat(readFlag(args, "--speed"), 4.6),
    seamProbeStrideFrames: readPositiveInt(readFlag(args, "--seam-stride"), 15),
    captureStrideFrames: readPositiveInt(readFlag(args, "--capture-stride"), 999999),
    referenceDiffStrideFrames: readNonNegativeInt(readFlag(args, "--reference-diff-stride"), 0),
    referenceDiffLimit: readNonNegativeInt(readFlag(args, "--reference-diff-limit"), 0),
  };
}

function readFlag(args: readonly string[], flag: string): string | null {
  const exact = args.find((arg) => arg.startsWith(`${flag}=`));
  if (exact) {
    return exact.slice(flag.length + 1);
  }
  const index = args.indexOf(flag);
  if (index === -1) {
    return null;
  }
  return args[index + 1] ?? null;
}

function readBooleanFlag(args: readonly string[], flag: string, fallback: boolean): boolean {
  const raw = readFlag(args, flag);
  if (raw === null) {
    return fallback;
  }
  if (raw === "1" || raw === "true") {
    return true;
  }
  if (raw === "0" || raw === "false") {
    return false;
  }
  return fallback;
}

function readOptionalPositiveInt(raw: string | null): number | null {
  if (raw === null) {
    return null;
  }
  const parsed = Number.parseInt(raw, 10);
  return Number.isFinite(parsed) && parsed > 0 ? parsed : null;
}

function readPositiveInt(raw: string | null, fallback: number): number {
  const parsed = Number.parseInt(raw ?? "", 10);
  return Number.isFinite(parsed) && parsed > 0 ? parsed : fallback;
}

function readNonNegativeInt(raw: string | null, fallback: number): number {
  const parsed = Number.parseInt(raw ?? "", 10);
  return Number.isFinite(parsed) && parsed >= 0 ? parsed : fallback;
}

function readPositiveFloat(raw: string | null, fallback: number): number {
  const parsed = Number(raw);
  return Number.isFinite(parsed) && parsed > 0 ? parsed : fallback;
}

function resolveChromeBinary(): string {
  const candidates = [
    process.env.CHROME_BIN,
    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    Bun.which("google-chrome"),
    Bun.which("Google Chrome"),
    Bun.which("chromium"),
    Bun.which("chromium-browser"),
  ];
  for (const candidate of candidates) {
    if (candidate) {
      return candidate;
    }
  }
  throw new Error("Could not resolve a Chrome binary. Pass --chrome-binary=/path/to/chrome");
}

function buildChromeCommand(
  chromeBinary: string,
  devToolsPort: number,
  userDataDir: string,
  headless: boolean,
): string[] {
  return [
    chromeBinary,
    `--remote-debugging-port=${devToolsPort}`,
    `--user-data-dir=${userDataDir}`,
    "--no-first-run",
    "--no-default-browser-check",
    "--disable-background-networking",
    "--disable-component-update",
    "--disable-default-apps",
    "--mute-audio",
    ...(headless ? ["--headless=new"] : []),
    "about:blank",
  ];
}

function runCommand(command: string[]): CommandResult {
  const startedAt = performance.now();
  const result = Bun.spawnSync(command, {
    cwd: process.cwd(),
    stdout: "pipe",
    stderr: "pipe",
    env: process.env,
  });
  return {
    exitCode: result.exitCode,
    elapsedMs: Number((performance.now() - startedAt).toFixed(2)),
    stdout: decodeBytes(result.stdout),
    stderr: decodeBytes(result.stderr),
  };
}

async function findFreePort(): Promise<number> {
  return await new Promise<number>((resolve, reject) => {
    const server = createServer();
    server.unref();
    server.on("error", reject);
    server.listen(0, "127.0.0.1", () => {
      const address = server.address();
      if (!address || typeof address === "string") {
        server.close(() => reject(new Error("Failed to resolve a free port")));
        return;
      }
      const { port } = address;
      server.close((error) => {
        if (error) {
          reject(error);
          return;
        }
        resolve(port);
      });
    });
  });
}

async function waitForHttp(url: string, timeoutMs: number): Promise<void> {
  await pollUntil(async () => {
    const response = await fetch(url);
    if (!response.ok) {
      throw new Error(`Unexpected status ${response.status}`);
    }
  }, timeoutMs);
}

async function waitForJsonEndpoint<T>(url: string, timeoutMs: number): Promise<T> {
  let lastError: unknown = null;
  const value = await pollUntil(async () => {
    try {
      const response = await fetch(url);
      if (!response.ok) {
        throw new Error(`Unexpected status ${response.status}`);
      }
      return await response.json() as T;
    } catch (error) {
      lastError = error;
      throw error;
    }
  }, timeoutMs);
  if (value === undefined) {
    throw lastError instanceof Error ? lastError : new Error("Timed out waiting for JSON endpoint");
  }
  return value;
}

async function createDevToolsTarget(port: number): Promise<DevToolsTargetResponse> {
  const endpoint = `http://127.0.0.1:${port}/json/new?about:blank`;
  const response = await fetch(endpoint, { method: "PUT" });
  if (response.ok) {
    return await response.json() as DevToolsTargetResponse;
  }
  const fallback = await fetch(endpoint);
  if (!fallback.ok) {
    throw new Error(`Failed to create DevTools target: ${response.status} / ${fallback.status}`);
  }
  return await fallback.json() as DevToolsTargetResponse;
}

async function waitForGameReady(cdp: CdpConnection, timeoutMs: number): Promise<void> {
  await pollUntil(async () => {
    const ready = await cdp.evaluate<boolean>([
      "(() => {",
      "  if (typeof window.__VOXELS_GAME__ !== 'object' || window.__VOXELS_GAME__ === null) {",
      "    return false;",
      "  }",
      "  const snapshot = window.__VOXELS_GAME__.snapshot();",
      "  return Boolean(",
      "    snapshot",
      "    && typeof snapshot.chunkCount === 'number'",
      "    && snapshot.chunkCount > 0",
      "    && snapshot.bootstrapPlayableReady === true",
      "  );",
      "})()",
    ].join("\n"));
    if (!ready) {
      throw new Error("Game runtime not ready yet");
    }
  }, timeoutMs);
}

async function writeProtocolStreamToFile(cdp: CdpConnection, handle: string, filePath: string): Promise<void> {
  const chunks: Uint8Array[] = [];
  while (true) {
    const chunk = await cdp.send("IO.read", { handle });
    const data = typeof chunk.data === "string" ? chunk.data : "";
    const base64Encoded = chunk.base64Encoded === true;
    if (data) {
      chunks.push(
        base64Encoded
          ? Uint8Array.from(Buffer.from(data, "base64"))
          : new TextEncoder().encode(data),
      );
    }
    if (chunk.eof === true) {
      break;
    }
  }
  await cdp.send("IO.close", { handle });
  const totalLength = chunks.reduce((sum, chunk) => sum + chunk.length, 0);
  const merged = new Uint8Array(totalLength);
  let offset = 0;
  for (const chunk of chunks) {
    merged.set(chunk, offset);
    offset += chunk.length;
  }
  await Bun.write(filePath, merged);
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

function decodeBytes(bytes: Uint8Array): string {
  return new TextDecoder().decode(bytes);
}

function timestampForFile(date: Date): string {
  return date.toISOString().replace(/[:-]/g, "").replace(/\.\d{3}Z$/, "Z");
}

function sanitizeFileStem(value: string): string {
  return value.replace(/[^A-Za-z0-9._-]+/g, "-").replace(/-+/g, "-").replace(/^-|-$/g, "");
}

function readGitShortHead(): string | null {
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

function readStringField(value: Record<string, unknown>, key: string): string | null {
  const field = value[key];
  return typeof field === "string" ? field : null;
}

function printReportSummary(reportPath: string, report: BrowserRouteTraceReport): void {
  const benchmarkSummary = (report.benchmark.summary ?? {}) as Record<string, unknown>;
  const discoveryLandmarks = (report.discovery.discoveredLandmarkIds ?? []) as unknown[];
  console.log([
    `browser-route-trace report: ${reportPath}`,
    `app url: ${report.appUrl}`,
    `trace path: ${report.tracePath}`,
    `avg gameplay frame: ${formatNumber(benchmarkSummary.avgGameplayFrameMs)} ms`,
    `p95 gameplay frame: ${formatNumber(benchmarkSummary.p95GameplayFrameMs)} ms`,
    `frames with hole signals: ${formatNumber(benchmarkSummary.framesWithHoleSignals)}`,
    `discovered landmark families: ${discoveryLandmarks.length}`,
  ].join("\n"));
}

function formatNumber(value: unknown): string {
  return typeof value === "number" ? value.toFixed(2) : "n/a";
}

async function connectCdp(url: string): Promise<CdpConnection> {
  return await CdpClient.connect(url);
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
