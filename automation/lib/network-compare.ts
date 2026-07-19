export interface NetworkScenarioSummary {
  readonly viewportFullyInformedMs: { readonly median: number; readonly max: number };
  readonly fullCoverageSettledMs: { readonly median: number };
  readonly bytesAtViewportInformed: { readonly medianWorldDownstream: number };
  readonly bytesAtFullCoverage: { readonly medianTotal: number };
}

export interface NetworkBenchmarkResult {
  readonly schemaVersion: number;
  readonly browserSnapshotSchema: unknown;
  readonly fixture: unknown;
  readonly protocol: unknown;
  readonly link: unknown;
  readonly world: unknown;
  readonly environment: unknown;
  repetitions: number;
  readonly summary: Record<string, NetworkScenarioSummary>;
  readonly git: { readonly commit: string };
}

export interface MetricDelta {
  readonly before: number;
  readonly after: number;
  readonly delta: number;
  readonly percent: number | null;
}

export interface NetworkScenarioComparison {
  readonly viewportMedianMs: MetricDelta;
  readonly viewportMaxMs: MetricDelta;
  readonly fullCoverageMedianMs: MetricDelta;
  readonly viewportWorldBytes: MetricDelta;
  readonly fullCoverageBytes: MetricDelta;
}

export type NetworkBenchmarkComparison = Record<string, NetworkScenarioComparison>;

function percentage(delta: number, baseline: number): number | null {
  return baseline === 0 ? null : (delta / baseline) * 100;
}

function metric(before: number, after: number): MetricDelta {
  const delta = after - before;
  return { before, after, delta, percent: percentage(delta, before) };
}

function requireSameFixture(
  baseline: NetworkBenchmarkResult,
  candidate: NetworkBenchmarkResult,
  field: keyof Pick<
    NetworkBenchmarkResult,
    "browserSnapshotSchema" | "fixture" | "protocol" | "link" | "world" | "environment"
  >,
): void {
  if (JSON.stringify(baseline[field]) !== JSON.stringify(candidate[field])) {
    throw new Error(`${field} mismatch`);
  }
}

export function compareNetworkBenchmarks(
  baseline: NetworkBenchmarkResult,
  candidate: NetworkBenchmarkResult,
): NetworkBenchmarkComparison {
  if (baseline.schemaVersion !== candidate.schemaVersion) {
    throw new Error(
      `result schema mismatch: ${baseline.schemaVersion} versus ${candidate.schemaVersion}`,
    );
  }
  requireSameFixture(baseline, candidate, "browserSnapshotSchema");
  requireSameFixture(baseline, candidate, "fixture");
  requireSameFixture(baseline, candidate, "protocol");
  requireSameFixture(baseline, candidate, "link");
  requireSameFixture(baseline, candidate, "world");
  requireSameFixture(baseline, candidate, "environment");
  if (baseline.repetitions !== candidate.repetitions) {
    throw new Error(
      `repetitions mismatch: ${baseline.repetitions} versus ${candidate.repetitions}`,
    );
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
      if (before === undefined || after === undefined) {
        throw new Error(`scenario ${name} is missing after set validation`);
      }
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

function signed(value: number, digits = 1): string {
  return `${value >= 0 ? "+" : ""}${value.toFixed(digits)}`;
}

function formatDelta(value: MetricDelta, suffix = ""): string {
  const percent = value.percent === null ? "n/a" : `${signed(value.percent)}%`;
  return `${signed(value.delta)}${suffix} (${percent})`;
}

export function comparisonMarkdown(
  baseline: NetworkBenchmarkResult,
  candidate: NetworkBenchmarkResult,
  comparison: NetworkBenchmarkComparison,
): string {
  const rows = Object.entries(comparison).map(
    ([name, values]) =>
      `| ${name} | ${formatDelta(values.viewportMedianMs, " ms")} | ${formatDelta(values.viewportMaxMs, " ms")} | ${formatDelta(values.fullCoverageMedianMs, " ms")} | ${formatDelta(values.viewportWorldBytes, " B")} | ${formatDelta(values.fullCoverageBytes, " B")} |`,
  );
  return `# Network benchmark comparison\n\nBaseline: \`${baseline.git.commit}\`  \nCandidate: \`${candidate.git.commit}\`\n\nNegative values are improvements; positive values are degradations.\n\n| Scenario | Viewport median | Viewport max | Full coverage median | World bytes at viewport | Total bytes at full coverage |\n| --- | ---: | ---: | ---: | ---: | ---: |\n${rows.join("\n")}\n`;
}
