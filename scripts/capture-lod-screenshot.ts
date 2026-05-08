import { mkdir } from "node:fs/promises";
import { join } from "node:path";
import { withBrowserGameSession } from "./lib/browser-game-benchmark-harness.ts";

function timestampForFile(date: Date): string {
  return date.toISOString().replace(/[-:]/g, "").replace(/\.\d{3}Z$/, "Z");
}

function readFlag(args: readonly string[], name: string): string | null {
  const prefix = `${name}=`;
  const match = args.find((arg) => arg.startsWith(prefix));
  return match ? match.slice(prefix.length) : null;
}

const args = Bun.argv.slice(2);
const settleMs = Number(readFlag(args, "--settle-ms") ?? 15_000);
const waitForLod = readFlag(args, "--wait-for-lod") !== "false";
const outputDir = join("artifacts", "manual-screenshots", `${timestampForFile(new Date())}-lod-screenshot`);
await mkdir(outputDir, { recursive: true });

await withBrowserGameSession({
  outputDir,
  viewportWidth: 1440,
  viewportHeight: 900,
}, async (session) => {
  await session.navigateToGame({
    clearStorage: true,
    query: { benchmarkBootstrap: 1 },
  });
  await session.waitForGameReady(120_000);
  const settleStartedAt = performance.now();
  while (performance.now() - settleStartedAt < Math.max(0, settleMs)) {
    await session.evaluate("new Promise((resolve) => setTimeout(resolve, 1000))");
    if (!waitForLod) {
      continue;
    }
    const pending = await session.evaluate<number>("window.__VOXELS_GAME__.snapshot().lodPendingChunks");
    if (pending === 0) {
      break;
    }
  }
  const snapshot = await session.evaluate<Record<string, unknown>>("window.__VOXELS_GAME__.snapshot()");
  const screenshot = await session.captureScreenshotPng();
  const screenshotPath = join(outputDir, "viewport.png");
  const snapshotPath = join(outputDir, "snapshot.json");
  await Bun.write(screenshotPath, screenshot);
  await Bun.write(snapshotPath, `${JSON.stringify(snapshot, null, 2)}\n`);
  console.log(JSON.stringify({ screenshotPath, snapshotPath, snapshot }, null, 2));
});
