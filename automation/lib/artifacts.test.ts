import { mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { afterEach, describe, expect, it } from "vite-plus/test";
import { ArtifactStore } from "./artifacts.ts";

const temporaryDirectories: string[] = [];

afterEach(async () => {
  await Promise.all(
    temporaryDirectories
      .splice(0)
      .map((directory) => rm(directory, { recursive: true, force: true })),
  );
});

describe("automation artifacts", () => {
  it("allocates unique directories for concurrent runs", async () => {
    const root = await mkdtemp(path.join(tmpdir(), "voxels-artifacts-"));
    temporaryDirectories.push(root);
    const [left, right] = await Promise.all([
      ArtifactStore.create("example", { root }),
      ArtifactStore.create("example", { root }),
    ]);

    expect(left.runId).not.toBe(right.runId);
    expect(left.directory).not.toBe(right.directory);
  });

  it("publishes a stable pointer without flattening run artifacts", async () => {
    const root = await mkdtemp(path.join(tmpdir(), "voxels-artifacts-"));
    temporaryDirectories.push(root);
    const store = await ArtifactStore.create("example", { root, runId: "run-1" });
    await store.writeJson("manifest", "manifest.json", { status: "passed" });
    await store.publishLatest("passed");
    const latest = JSON.parse(
      await readFile(path.join(root, "example", "latest.json"), "utf8"),
    ) as { runId: string; status: string; manifest: string };
    expect(latest).toEqual({
      runId: "run-1",
      status: "passed",
      schemaVersion: 1,
      directory: path.join(root, "example", "run-1"),
      manifest: path.join(root, "example", "run-1", "manifest.json"),
    });
  });
});
