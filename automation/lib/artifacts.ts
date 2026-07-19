import { copyFile, mkdir, writeFile } from "node:fs/promises";
import path from "node:path";

export interface ArtifactRecord {
  readonly label: string;
  readonly path: string;
  readonly mediaType?: string;
}

export interface ArtifactOptions {
  readonly root?: string;
  readonly runId?: string;
}

function safeSegment(value: string): string {
  const normalized = value.trim().replaceAll(/[^a-zA-Z0-9._-]+/gu, "-");
  if (normalized.length === 0 || normalized === "." || normalized === "..") {
    throw new Error(`invalid artifact path segment ${JSON.stringify(value)}`);
  }
  return normalized;
}

function timestampRunId(now: Date): string {
  return now.toISOString().replaceAll(/[:.]/gu, "-");
}

export class ArtifactStore {
  readonly directory: string;
  readonly records: ArtifactRecord[] = [];

  private constructor(directory: string) {
    this.directory = directory;
  }

  static async create(scenarioId: string, options: ArtifactOptions = {}): Promise<ArtifactStore> {
    const root = path.resolve(
      options.root ?? process.env.VOXELS_AUTOMATION_OUTPUT ?? "target/automation",
    );
    const runId = safeSegment(options.runId ?? timestampRunId(new Date()));
    const directory = path.join(root, safeSegment(scenarioId), runId);
    await mkdir(directory, { recursive: true });
    return new ArtifactStore(directory);
  }

  resolve(...segments: readonly string[]): string {
    const resolved = path.join(this.directory, ...segments.map(safeSegment));
    const relative = path.relative(this.directory, resolved);
    if (relative.startsWith("..") || path.isAbsolute(relative)) {
      throw new Error("artifact path escaped its scenario run directory");
    }
    return resolved;
  }

  async directoryFor(...segments: readonly string[]): Promise<string> {
    const directory = this.resolve(...segments);
    await mkdir(directory, { recursive: true });
    return directory;
  }

  record(label: string, artifactPath: string, mediaType?: string): string {
    const absolute = path.resolve(artifactPath);
    const relative = path.relative(this.directory, absolute);
    if (relative.startsWith("..") || path.isAbsolute(relative)) {
      throw new Error(`artifact ${absolute} is outside ${this.directory}`);
    }
    this.records.push(
      Object.freeze({
        label,
        path: absolute,
        ...(mediaType === undefined ? {} : { mediaType }),
      }),
    );
    return absolute;
  }

  async writeJson(label: string, filename: string, value: unknown): Promise<string> {
    const destination = this.resolve(filename);
    await writeFile(destination, `${JSON.stringify(value, null, 2)}\n`);
    return this.record(label, destination, "application/json");
  }

  async writeText(
    label: string,
    filename: string,
    value: string,
    mediaType = "text/plain",
  ): Promise<string> {
    const destination = this.resolve(filename);
    await writeFile(destination, value);
    return this.record(label, destination, mediaType);
  }

  async copy(label: string, source: string, filename: string, mediaType?: string): Promise<string> {
    const destination = this.resolve(filename);
    await copyFile(source, destination);
    return this.record(label, destination, mediaType);
  }
}
