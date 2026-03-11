interface TraceEvent {
  pid: number;
  tid: number;
  ts: number;
  ph?: string;
  dur?: number;
  name: string;
  args?: {
    name?: string;
    data?: {
      cpuProfile?: {
        nodes?: CpuProfileNode[];
        samples?: number[];
      };
      timeDeltas?: number[];
      functionName?: string;
      jsHeapSizeUsed?: number;
      lineNumber?: number;
      url?: string;
    };
  };
}

interface CpuProfileNode {
  id: number;
  parent?: number;
  callFrame?: {
    codeType?: string;
    columnNumber?: number;
    functionName?: string;
    lineNumber?: number;
    url?: string;
  };
}

interface CliOptions {
  tracePath: string;
  urlPrefix: string | null;
  longTaskThresholdMs: number;
}

function parseCli(argv: readonly string[]): CliOptions {
  const args = argv.slice(2);
  let tracePath = "";
  let urlPrefix: string | null = null;
  let longTaskThresholdMs = 8;
  for (const arg of args) {
    if (arg.startsWith("--url-prefix=")) {
      urlPrefix = arg.slice("--url-prefix=".length);
      continue;
    }
    if (arg.startsWith("--long-task-ms=")) {
      const parsed = Number(arg.slice("--long-task-ms=".length));
      if (Number.isFinite(parsed) && parsed > 0) {
        longTaskThresholdMs = parsed;
      }
      continue;
    }
    if (!tracePath) {
      tracePath = arg;
      continue;
    }
    throw new Error(`Unexpected argument: ${arg}`);
  }
  if (!tracePath) {
    throw new Error("Usage: bun run scripts/analyze-chrome-trace.ts <trace.json> [--url-prefix=http://localhost:3021/] [--long-task-ms=8]");
  }
  return { tracePath, urlPrefix, longTaskThresholdMs };
}

function normalizeUrl(url: string | undefined, urlPrefix: string | null): string | null {
  if (!url || url.startsWith("pptr:")) {
    return null;
  }
  if (!urlPrefix) {
    return url;
  }
  return url.startsWith(urlPrefix) ? url : null;
}

function selectTargetPid(events: readonly TraceEvent[], urlPrefix: string | null): {
  pid: number;
  urls: string[];
} {
  const urlsByPid = new Map<number, Set<string>>();
  for (const event of events) {
    if (event.name !== "ProfileChunk") {
      continue;
    }
    const nodes = event.args?.data?.cpuProfile?.nodes ?? [];
    for (const node of nodes) {
      const normalizedUrl = normalizeUrl(node.callFrame?.url, urlPrefix);
      if (!normalizedUrl) {
        continue;
      }
      let urls = urlsByPid.get(event.pid);
      if (!urls) {
        urls = new Set<string>();
        urlsByPid.set(event.pid, urls);
      }
      urls.add(normalizedUrl);
    }
  }
  const matches = [...urlsByPid.entries()];
  if (matches.length === 0) {
    throw new Error(`No ProfileChunk URLs matched${urlPrefix ? ` ${urlPrefix}` : ""}`);
  }
  if (matches.length > 1) {
    const withGame = matches.find(([, urls]) => [...urls].some((url) => url.includes("/game") || url.includes("client/game")));
    if (withGame) {
      return { pid: withGame[0], urls: [...withGame[1]].sort() };
    }
  }
  const [pid, urls] = matches[0]!;
  return { pid, urls: [...urls].sort() };
}

function formatFrameKey(node: CpuProfileNode | undefined, urlPrefix: string | null): string {
  const callFrame = node?.callFrame ?? {};
  const rawUrl = callFrame.url ?? "";
  const url = urlPrefix && rawUrl.startsWith(urlPrefix)
    ? rawUrl.slice(urlPrefix.length)
    : rawUrl;
  const line = callFrame.lineNumber ?? "";
  return `${callFrame.functionName || "(anonymous)"} @ ${url}:${line}`;
}

function summarizeCpuProfile(
  events: readonly TraceEvent[],
  pid: number,
  urlPrefix: string | null,
): {
  topExclusive: Array<{ frame: string; ms: number }>;
  topInclusive: Array<{ frame: string; ms: number }>;
} {
  const chunks = events
    .filter((event) => event.name === "ProfileChunk" && event.pid === pid)
    .sort((left, right) => left.ts - right.ts);
  const nodes = new Map<number, CpuProfileNode>();
  for (const chunk of chunks) {
    for (const node of chunk.args?.data?.cpuProfile?.nodes ?? []) {
      nodes.set(node.id, node);
    }
  }
  const exclusive = new Map<string, number>();
  const inclusive = new Map<string, number>();
  for (const chunk of chunks) {
    const samples = chunk.args?.data?.cpuProfile?.samples ?? [];
    const timeDeltas = chunk.args?.data?.timeDeltas ?? [];
    for (let index = 0; index < samples.length; index += 1) {
      const nodeId = samples[index]!;
      const delta = timeDeltas[index] ?? 0;
      const exclusiveKey = formatFrameKey(nodes.get(nodeId), urlPrefix);
      exclusive.set(exclusiveKey, (exclusive.get(exclusiveKey) ?? 0) + delta);
      let currentNodeId: number | undefined = nodeId;
      const seen = new Set<number>();
      while (currentNodeId !== undefined && !seen.has(currentNodeId)) {
        seen.add(currentNodeId);
        const inclusiveKey = formatFrameKey(nodes.get(currentNodeId), urlPrefix);
        inclusive.set(inclusiveKey, (inclusive.get(inclusiveKey) ?? 0) + delta);
        currentNodeId = nodes.get(currentNodeId)?.parent;
      }
    }
  }
  const toRows = (values: Map<string, number>) => [...values.entries()]
    .sort((left, right) => right[1] - left[1])
    .slice(0, 25)
    .map(([frame, microseconds]) => ({
      frame,
      ms: Number((microseconds / 1000).toFixed(1)),
    }));
  return {
    topExclusive: toRows(exclusive),
    topInclusive: toRows(inclusive),
  };
}

function summarizeLongTasks(
  events: readonly TraceEvent[],
  pid: number,
  thresholdMs: number,
): Array<{ tsMs: number; ms: number }> {
  const mainThread = events.find((event) =>
    event.pid === pid
    && event.ph === "M"
    && event.name === "thread_name"
    && event.args?.name === "CrRendererMain");
  if (!mainThread) {
    return [];
  }
  return events
    .filter((event) =>
      event.pid === pid
      && event.tid === mainThread.tid
      && event.name === "RunTask"
      && typeof event.dur === "number"
      && event.dur >= thresholdMs * 1000)
    .sort((left, right) => (right.dur ?? 0) - (left.dur ?? 0))
    .slice(0, 20)
    .map((event) => ({
      tsMs: Number((event.ts / 1000).toFixed(1)),
      ms: Number(((event.dur ?? 0) / 1000).toFixed(1)),
    }));
}

function summarizeHeap(
  events: readonly TraceEvent[],
  pid: number,
): {
  samples: number;
  minMB: number;
  maxMB: number;
  startMB: number;
  endMB: number;
  deltaMB: number;
  biggestRiseMB: number;
  biggestRiseStartMs: number;
  biggestRiseEndMs: number;
  biggestDropMB: number;
  biggestDropStartMs: number;
  biggestDropEndMs: number;
} | null {
  const counters = events.filter((event) =>
    event.pid === pid
    && event.name === "UpdateCounters"
    && typeof event.args?.data?.jsHeapSizeUsed === "number");
  if (counters.length === 0) {
    return null;
  }
  const sizes = counters.map((event) => event.args!.data!.jsHeapSizeUsed!);
  let biggestRise = Number.NEGATIVE_INFINITY;
  let biggestRiseStartMs = 0;
  let biggestRiseEndMs = 0;
  let biggestDrop = Number.POSITIVE_INFINITY;
  let biggestDropStartMs = 0;
  let biggestDropEndMs = 0;
  for (let index = 1; index < counters.length; index += 1) {
    const previous = counters[index - 1]!;
    const current = counters[index]!;
    const delta = current.args!.data!.jsHeapSizeUsed! - previous.args!.data!.jsHeapSizeUsed!;
    if (delta > biggestRise) {
      biggestRise = delta;
      biggestRiseStartMs = previous.ts / 1000;
      biggestRiseEndMs = current.ts / 1000;
    }
    if (delta < biggestDrop) {
      biggestDrop = delta;
      biggestDropStartMs = previous.ts / 1000;
      biggestDropEndMs = current.ts / 1000;
    }
  }
  return {
    samples: counters.length,
    minMB: Number((Math.min(...sizes) / 1048576).toFixed(2)),
    maxMB: Number((Math.max(...sizes) / 1048576).toFixed(2)),
    startMB: Number((sizes[0]! / 1048576).toFixed(2)),
    endMB: Number((sizes[sizes.length - 1]! / 1048576).toFixed(2)),
    deltaMB: Number(((sizes[sizes.length - 1]! - sizes[0]!) / 1048576).toFixed(2)),
    biggestRiseMB: Number((biggestRise / 1048576).toFixed(2)),
    biggestRiseStartMs: Number(biggestRiseStartMs.toFixed(1)),
    biggestRiseEndMs: Number(biggestRiseEndMs.toFixed(1)),
    biggestDropMB: Number((biggestDrop / 1048576).toFixed(2)),
    biggestDropStartMs: Number(biggestDropStartMs.toFixed(1)),
    biggestDropEndMs: Number(biggestDropEndMs.toFixed(1)),
  };
}

function summarizeGc(events: readonly TraceEvent[], pid: number): Array<{ name: string; count: number; totalMs: number; maxMs: number }> {
  const gcEvents = events.filter((event) =>
    event.pid === pid
    && (
      event.name === "MinorGC"
      || event.name === "MajorGC"
      || event.name === "V8.GCScavenger"
      || event.name === "V8.GCFinalizeMC"
    ));
  const summary = new Map<string, { count: number; total: number; max: number }>();
  for (const event of gcEvents) {
    const row = summary.get(event.name) ?? { count: 0, total: 0, max: 0 };
    row.count += 1;
    row.total += event.dur ?? 0;
    row.max = Math.max(row.max, event.dur ?? 0);
    summary.set(event.name, row);
  }
  return [...summary.entries()]
    .sort((left, right) => right[1].total - left[1].total)
    .map(([name, row]) => ({
      name,
      count: row.count,
      totalMs: Number((row.total / 1000).toFixed(3)),
      maxMs: Number((row.max / 1000).toFixed(3)),
    }));
}

export {};

const { tracePath, urlPrefix, longTaskThresholdMs } = parseCli(Bun.argv);
const traceJson = await Bun.file(tracePath).text();
const trace = JSON.parse(traceJson) as { traceEvents: TraceEvent[] };
const selected = selectTargetPid(trace.traceEvents, urlPrefix);
const cpu = summarizeCpuProfile(trace.traceEvents, selected.pid, urlPrefix);
const heap = summarizeHeap(trace.traceEvents, selected.pid);
const gcSummary = summarizeGc(trace.traceEvents, selected.pid);
const longTasks = summarizeLongTasks(trace.traceEvents, selected.pid, longTaskThresholdMs);

console.log(JSON.stringify({
  tracePath,
  pid: selected.pid,
  urls: selected.urls,
  topExclusive: cpu.topExclusive,
  topInclusive: cpu.topInclusive,
  heap,
  gc: gcSummary,
  longTasks,
}, null, 2));
