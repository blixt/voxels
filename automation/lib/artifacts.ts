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
  readonly scenarioDirectory: string;
  readonly runId: string;

  readonly #records: ArtifactRecord[] = [];
  #sealed = false;

  private constructor(directory: string, scenarioDirectory: string, runId: string) {
    this.directory = directory;
    this.scenarioDirectory = scenarioDirectory;
    this.runId = runId;
  }

  static async create(scenarioId: string, options: ArtifactOptions = {}): Promise<ArtifactStore> {
    const root = path.resolve(
      options.root ?? process.env.VOXELS_AUTOMATION_OUTPUT ?? "target/automation",
    );
    const runId = safeSegment(options.runId ?? timestampRunId(new Date()));
    const scenarioDirectory = path.join(root, safeSegment(scenarioId));
    const directory = path.join(scenarioDirectory, runId);
    await mkdir(directory, { recursive: true });
    return new ArtifactStore(directory, scenarioDirectory, runId);
  }

  get records(): readonly ArtifactRecord[] {
    return Object.freeze([...this.#records]);
  }

  seal(): void {
    this.#sealed = true;
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
    this.#assertWritable();
    const absolute = path.resolve(artifactPath);
    const relative = path.relative(this.directory, absolute);
    if (relative.startsWith("..") || path.isAbsolute(relative)) {
      throw new Error(`artifact ${absolute} is outside ${this.directory}`);
    }
    this.#records.push(
      Object.freeze({
        label,
        path: absolute,
        ...(mediaType === undefined ? {} : { mediaType }),
      }),
    );
    return absolute;
  }

  async writeJson(label: string, filename: string, value: unknown): Promise<string> {
    this.#assertWritable();
    const destination = this.resolve(filename);
    await writeFile(destination, `${JSON.stringify(value, null, 2)}\n`);
    return this.record(label, destination, "application/json");
  }

  async writeMetadataJson(filename: string, value: unknown): Promise<string> {
    const destination = this.resolve(filename);
    await writeFile(destination, `${JSON.stringify(value, null, 2)}\n`);
    return destination;
  }

  async writeText(
    label: string,
    filename: string,
    value: string,
    mediaType = "text/plain",
  ): Promise<string> {
    this.#assertWritable();
    const destination = this.resolve(filename);
    await writeFile(destination, value);
    return this.record(label, destination, mediaType);
  }

  async write(
    label: string,
    filename: string,
    value: Uint8Array,
    mediaType?: string,
  ): Promise<string> {
    this.#assertWritable();
    const destination = this.resolve(filename);
    await writeFile(destination, value);
    return this.record(label, destination, mediaType);
  }

  async copy(label: string, source: string, filename: string, mediaType?: string): Promise<string> {
    this.#assertWritable();
    const destination = this.resolve(filename);
    await copyFile(source, destination);
    return this.record(label, destination, mediaType);
  }

  async publishLatest(status: "passed" | "failed"): Promise<void> {
    await writeFile(
      path.join(this.scenarioDirectory, "latest.json"),
      `${JSON.stringify(
        {
          schemaVersion: 1,
          runId: this.runId,
          status,
          directory: this.directory,
          manifest: this.resolve("manifest.json"),
        },
        null,
        2,
      )}\n`,
    );
  }

  #assertWritable(): void {
    if (this.#sealed) throw new Error("automation artifacts are sealed");
  }
}
