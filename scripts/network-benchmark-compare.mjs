import { readFile } from "node:fs/promises";
import { pathToFileURL } from "node:url";

function percentage(delta, baseline) {
  return baseline === 0 ? null : (delta / baseline) * 100;
}

function metric(before, after) {
  const delta = after - before;
  return { before, after, delta, percent: percentage(delta, before) };
}

export function compareNetworkBenchmarks(baseline, candidate) {
  if (baseline.schemaVersion !== candidate.schemaVersion) {
    throw new Error(
      `result schema mismatch: ${baseline.schemaVersion} versus ${candidate.schemaVersion}`,
    );
  }
  for (const field of [
    "roundTripLatencyMs",
    "downstreamMegabitsPerSecond",
    "upstreamMegabitsPerSecond",
    "quantumBytes",
  ]) {
    if (baseline.link[field] !== candidate.link[field]) {
      throw new Error(`link profile mismatch for ${field}`);
    }
  }
  const commonScenarios = Object.keys(baseline.summary).filter((name) => candidate.summary[name]);
  if (commonScenarios.length === 0) throw new Error("benchmark results have no common scenarios");
  return Object.fromEntries(
    commonScenarios.map((name) => {
      const before = baseline.summary[name];
      const after = candidate.summary[name];
      return [
        name,
        {
          viewportMedianMs: metric(
            before.viewportFullyInformedMs.median,
            after.viewportFullyInformedMs.median,
          ),
          viewportP95Ms: metric(
            before.viewportFullyInformedMs.p95,
            after.viewportFullyInformedMs.p95,
          ),
          fullCoverageMedianMs: metric(
            before.fullCoverageSettledMs.median,
            after.fullCoverageSettledMs.median,
          ),
          streamBytes: metric(before.streamBytes.medianTotal, after.streamBytes.medianTotal),
        },
      ];
    }),
  );
}

function signed(value, digits = 1) {
  return `${value >= 0 ? "+" : ""}${value.toFixed(digits)}`;
}

function formatDelta(value, suffix = "") {
  const percent = value.percent === null ? "n/a" : `${signed(value.percent)}%`;
  return `${signed(value.delta)}${suffix} (${percent})`;
}

export function comparisonMarkdown(baseline, candidate, comparison) {
  const rows = Object.entries(comparison).map(
    ([name, values]) =>
      `| ${name} | ${formatDelta(values.viewportMedianMs, " ms")} | ${formatDelta(values.viewportP95Ms, " ms")} | ${formatDelta(values.fullCoverageMedianMs, " ms")} | ${formatDelta(values.streamBytes, " B")} |`,
  );
  return `# Network benchmark comparison\n\nBaseline: \`${baseline.git.commit}\`  \nCandidate: \`${candidate.git.commit}\`\n\nNegative values are improvements; positive values are degradations.\n\n| Scenario | Viewport median | Viewport p95 | Full coverage median | Stream bytes |\n| --- | ---: | ---: | ---: | ---: |\n${rows.join("\n")}\n`;
}

async function main() {
  const paths = process.argv.slice(2).filter((argument) => argument !== "--");
  if (paths.length !== 2) {
    throw new Error("usage: vp run bench:network:compare -- <baseline.json> <candidate.json>");
  }
  const [baseline, candidate] = await Promise.all(
    paths.map(async (file) => JSON.parse(await readFile(file, "utf8"))),
  );
  const comparison = compareNetworkBenchmarks(baseline, candidate);
  process.stdout.write(comparisonMarkdown(baseline, candidate, comparison));
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) await main();
