import { execFile } from "node:child_process";
import { promisify } from "node:util";

const execFileAsync = promisify(execFile);

export interface NumericSummary {
  readonly samples: number;
  readonly min: number;
  readonly p50: number;
  readonly p95: number;
  readonly p99: number;
  readonly max: number;
  readonly mean: number;
}

export interface ProcessSample {
  readonly atUnixMs: number;
  readonly cpuPercent: number;
  readonly rssBytes: number;
  readonly virtualBytes: number;
  readonly threads: number | null;
}

export function percentile(values: readonly number[], fraction: number): number {
  if (values.length === 0) return 0;
  const sorted = [...values].sort((left, right) => left - right);
  const index = Math.min(sorted.length - 1, Math.max(0, Math.ceil(sorted.length * fraction) - 1));
  return sorted[index] ?? 0;
}

export function numericSummary(values: readonly number[], digits = 3): NumericSummary {
  if (values.length === 0) {
    return { samples: 0, min: 0, p50: 0, p95: 0, p99: 0, max: 0, mean: 0 };
  }
  const round = (value: number): number => {
    const scale = 10 ** digits;
    return Math.round(value * scale) / scale;
  };
  return {
    samples: values.length,
    min: round(Math.min(...values)),
    p50: round(percentile(values, 0.5)),
    p95: round(percentile(values, 0.95)),
    p99: round(percentile(values, 0.99)),
    max: round(Math.max(...values)),
    mean: round(values.reduce((sum, value) => sum + value, 0) / values.length),
  };
}

export async function sampleProcess(pid: number): Promise<ProcessSample | null> {
  try {
    const { stdout } = await execFileAsync("ps", ["-o", "pcpu=,rss=,vsz=", "-p", String(pid)]);
    const values = stdout.trim().split(/\s+/u).map(Number);
    const [cpuPercent, rssKiB, virtualKiB] = values;
    if (
      values.length !== 3 ||
      cpuPercent === undefined ||
      rssKiB === undefined ||
      virtualKiB === undefined ||
      values.some((value) => !Number.isFinite(value))
    ) {
      return null;
    }
    const threads = await processThreadCount(pid);
    return {
      atUnixMs: Date.now(),
      cpuPercent,
      rssBytes: rssKiB * 1_024,
      virtualBytes: virtualKiB * 1_024,
      threads,
    };
  } catch {
    return null;
  }
}

export function summarizeProcess(samples: readonly ProcessSample[]): {
  readonly samples: number;
  readonly cpuPercent: NumericSummary;
  readonly rssMiB: NumericSummary;
  readonly virtualMiB: NumericSummary;
  readonly threads: NumericSummary | null;
} {
  const summary = {
    samples: samples.length,
    cpuPercent: numericSummary(samples.map((sample) => sample.cpuPercent)),
    rssMiB: numericSummary(samples.map((sample) => sample.rssBytes / 1_048_576)),
    virtualMiB: numericSummary(samples.map((sample) => sample.virtualBytes / 1_048_576)),
  };
  const threadSamples = samples
    .map((sample) => sample.threads)
    .filter((threads): threads is number => threads !== null && Number.isFinite(threads));
  return {
    ...summary,
    threads: threadSamples.length > 0 ? numericSummary(threadSamples) : null,
  };
}

async function processThreadCount(pid: number): Promise<number | null> {
  try {
    if (process.platform === "darwin") {
      const { stdout } = await execFileAsync("ps", ["-M", "-p", String(pid)]);
      return Math.max(0, stdout.trim().split("\n").length - 1);
    }
    const { stdout } = await execFileAsync("ps", ["-o", "nlwp=", "-p", String(pid)]);
    const count = Number(stdout.trim());
    return Number.isFinite(count) ? count : null;
  } catch {
    return null;
  }
}
