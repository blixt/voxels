import { readFile } from "node:fs/promises";
import { pathToFileURL } from "node:url";

function percentage(delta, baseline) {
  return baseline === 0 ? null : (delta / baseline) * 100;
}

function metric(before, after) {
  const delta = after - before;
  return { before, after, delta, percent: percentage(delta, before) };
}

function requireSameFixture(baseline, candidate, field) {
  if (JSON.stringify(baseline[field]) !== JSON.stringify(candidate[field])) {
    throw new Error(`${field} mismatch`);
  }
}

export function compareNetworkBenchmarks(baseline, candidate) {
  if (baseline.schemaVersion !== candidate.schemaVersion) {
    throw new Error(
      `result schema mismatch: ${baseline.schemaVersion} versus ${candidate.schemaVersion}`,
    );
  }
  requireSameFixture(baseline, candidate, "browserSnapshotSchema");
  requireSameFixture(baseline, candidate, "fixture");
  requireSameFixture(baseline, candidate, "protocol");
  for (const field of [
    "roundTripLatencyMs",
    "downstreamMegabitsPerSecond",
    "upstreamMegabitsPerSecond",
    "quantumBytes",
    "upstreamMaxQueuedBytes",
    "downstreamMaxQueuedBytes",
  ]) {
    if (baseline.link[field] !== candidate.link[field]) {
      throw new Error(`link profile mismatch for ${field}`);
    }
  }
  const baselineScenarios = Object.keys(baseline.summary).toSorted();
  const candidateScenarios = Object.keys(candidate.summary).toSorted();
  if (JSON.stringify(baselineScenarios) !== JSON.stringify(candidateScenarios)) {
    throw new Error(
      `scenario set mismatch: ${baselineScenarios.join(", ")} versus ${candidateScenarios.join(", ")}`,
    );
  }
  if (baselineScenarios.length === 0) throw new Error("benchmark results have no scenarios");
  return Object.fromEntries(
    baselineScenarios.map((name) => {
      const before = baseline.summary[name];
      const after = candidate.summary[name];
      return [
        name,
        {
          viewportMedianMs: metric(
            before.viewportFullyInformedMs.median,
            after.viewportFullyInformedMs.median,
          ),
          viewportMaxMs: metric(
            before.viewportFullyInformedMs.max,
            after.viewportFullyInformedMs.max,
          ),
          fullCoverageMedianMs: metric(
            before.fullCoverageSettledMs.median,
            after.fullCoverageSettledMs.median,
          ),
          viewportWorldBytes: metric(
            before.bytesAtViewportInformed.medianWorldDownstream,
            after.bytesAtViewportInformed.medianWorldDownstream,
          ),
          fullCoverageBytes: metric(
            before.bytesAtFullCoverage.medianTotal,
            after.bytesAtFullCoverage.medianTotal,
          ),
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
      `| ${name} | ${formatDelta(values.viewportMedianMs, " ms")} | ${formatDelta(values.viewportMaxMs, " ms")} | ${formatDelta(values.fullCoverageMedianMs, " ms")} | ${formatDelta(values.viewportWorldBytes, " B")} | ${formatDelta(values.fullCoverageBytes, " B")} |`,
  );
  return `# Network benchmark comparison\n\nBaseline: \`${baseline.git.commit}\`  \nCandidate: \`${candidate.git.commit}\`\n\nNegative values are improvements; positive values are degradations.\n\n| Scenario | Viewport median | Viewport max | Full coverage median | World bytes at viewport | Total bytes at full coverage |\n| --- | ---: | ---: | ---: | ---: | ---: |\n${rows.join("\n")}\n`;
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
