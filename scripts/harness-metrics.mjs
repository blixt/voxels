import { execFile } from "node:child_process";
import { mkdir, writeFile } from "node:fs/promises";
import path from "node:path";
import { promisify } from "node:util";

const execFileAsync = promisify(execFile);

export function percentile(values, fraction) {
  if (values.length === 0) return 0;
  const sorted = [...values].sort((left, right) => left - right);
  const index = Math.min(sorted.length - 1, Math.max(0, Math.ceil(sorted.length * fraction) - 1));
  return sorted[index];
}

export function numericSummary(values, digits = 3) {
  if (values.length === 0) {
    return { samples: 0, min: 0, p50: 0, p95: 0, p99: 0, max: 0, mean: 0 };
  }
  const round = (value) => {
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

export async function sampleProcess(pid) {
  try {
    const { stdout } = await execFileAsync("ps", ["-o", "pcpu=,rss=,vsz=", "-p", String(pid)]);
    const values = stdout.trim().split(/\s+/u).map(Number);
    if (values.length !== 3 || values.some((value) => !Number.isFinite(value))) return null;
    const threads = await processThreadCount(pid);
    return {
      atUnixMs: Date.now(),
      cpuPercent: values[0],
      rssBytes: values[1] * 1_024,
      virtualBytes: values[2] * 1_024,
      threads,
    };
  } catch {
    return null;
  }
}

export function summarizeProcess(samples) {
  const summary = {
    samples: samples.length,
    cpuPercent: numericSummary(samples.map((sample) => sample.cpuPercent)),
    rssMiB: numericSummary(samples.map((sample) => sample.rssBytes / 1_048_576)),
    virtualMiB: numericSummary(samples.map((sample) => sample.virtualBytes / 1_048_576)),
  };
  const threadSamples = samples
    .map((sample) => sample.threads)
    .filter((threads) => Number.isFinite(threads));
  return {
    ...summary,
    threads: threadSamples.length > 0 ? numericSummary(threadSamples) : null,
  };
}

export async function writeHarnessReport(directory, result, markdown) {
  await mkdir(directory, { recursive: true });
  const timestamp = new Date().toISOString().replaceAll(":", "-");
  const json = `${JSON.stringify(result, null, 2)}\n`;
  await Promise.all([
    writeFile(path.join(directory, `${timestamp}.json`), json),
    writeFile(path.join(directory, `${timestamp}.md`), markdown),
    writeFile(path.join(directory, "latest.json"), json),
    writeFile(path.join(directory, "latest.md"), markdown),
  ]);
}

async function processThreadCount(pid) {
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
