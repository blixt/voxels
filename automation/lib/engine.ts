import type { Page } from "playwright";
import type { BrowserPlayerSession } from "../../web/local-player.ts";
import {
  FRAME_SAMPLE_WIDTH,
  GPU_SAMPLE_WIDTH,
  SNAPSHOT,
  SNAPSHOT_SCHEMA_VERSION,
  assertAutomationContract,
  assertSnapshotSchema,
  type AutomationEditShape,
  type EngineAutomationContract,
  type SnapshotField,
} from "../../web/automation.ts";

export {
  FRAME_SAMPLE_WIDTH,
  GPU_SAMPLE_WIDTH,
  SNAPSHOT,
  SNAPSHOT_SCHEMA_VERSION,
  assertSnapshotSchema,
  type EngineAutomationContract,
  type AutomationEditShape,
  type SnapshotField,
};

export interface SurfaceEditState {
  readonly tileX: number;
  readonly tileZ: number;
  readonly requiredServerRevision: number;
  readonly acceptedServerRevision: number;
  readonly resident: boolean;
  readonly dirty: boolean;
  readonly fingerprint: bigint;
  readonly quadCount: number;
  readonly activationMask: number;
}

export interface SnapshotWaitOptions {
  readonly timeoutMs?: number;
  readonly intervalMs?: number;
  readonly description?: string;
  readonly onSnapshot?: (snapshot: readonly number[]) => void;
}

export interface CameraLookOptions extends SnapshotWaitOptions {
  readonly sensitivity?: number;
  readonly tolerance?: number;
}

export class EngineClient {
  readonly #page: Page;
  #contract: EngineAutomationContract | undefined;

  constructor(page: Page) {
    this.#page = page;
  }

  async ready(timeoutMs = 30_000): Promise<EngineAutomationContract> {
    await this.#page.waitForFunction(
      () =>
        typeof globalThis.__VOXELS__?.contract === "function" &&
        typeof globalThis.__VOXELS__?.snapshot === "function",
      undefined,
      { timeout: timeoutMs },
    );
    const contract = await this.#page.evaluate(() => globalThis.__VOXELS__!.contract());
    assertAutomationContract(contract);
    this.#contract = contract;
    return contract;
  }

  async snapshot(): Promise<readonly number[]> {
    if (this.#contract === undefined) await this.ready();
    return assertSnapshotSchema(await this.#page.evaluate(() => globalThis.__VOXELS__!.snapshot()));
  }

  async value(field: SnapshotField): Promise<number> {
    const snapshot = await this.snapshot();
    const value = snapshot[SNAPSHOT[field]];
    if (value === undefined) throw new Error(`snapshot omitted ${field}`);
    return value;
  }

  async wait(milliseconds: number): Promise<void> {
    await this.#page.waitForTimeout(milliseconds);
  }

  async waitForSnapshot(
    predicate: (snapshot: readonly number[]) => boolean,
    {
      timeoutMs = 5_000,
      intervalMs = 25,
      description = "engine state did not settle",
      onSnapshot,
    }: SnapshotWaitOptions = {},
  ): Promise<readonly number[]> {
    const deadline = performance.now() + timeoutMs;
    let latest: readonly number[] = [];
    while (performance.now() < deadline) {
      latest = await this.snapshot();
      onSnapshot?.(latest);
      if (predicate(latest)) return latest;
      await this.wait(intervalMs);
    }
    throw new Error(`${description}: ${JSON.stringify(latest)}`);
  }

  async look(deltaX: number, deltaY: number): Promise<void> {
    await this.#page.evaluate(([x, y]) => globalThis.__VOXELS__!.look(x, y), [
      deltaX,
      deltaY,
    ] as const);
  }

  async setCameraLook(
    targetYaw: number,
    targetPitch: number,
    {
      sensitivity = 0.0022,
      tolerance = 0.001,
      description = "camera did not reach the requested look direction",
      ...waitOptions
    }: CameraLookOptions = {},
  ): Promise<readonly number[]> {
    const current = await this.snapshot();
    waitOptions.onSnapshot?.(current);
    const yawDelta = Math.atan2(
      Math.sin(targetYaw - snapshotValue(current, "yaw")),
      Math.cos(targetYaw - snapshotValue(current, "yaw")),
    );
    await this.look(
      yawDelta / sensitivity,
      (snapshotValue(current, "pitch") - targetPitch) / sensitivity,
    );
    return this.waitForSnapshot(
      (snapshot) => {
        const yawError = Math.atan2(
          Math.sin(snapshotValue(snapshot, "yaw") - targetYaw),
          Math.cos(snapshotValue(snapshot, "yaw") - targetYaw),
        );
        return (
          Math.abs(yawError) < tolerance &&
          Math.abs(snapshotValue(snapshot, "pitch") - targetPitch) < tolerance
        );
      },
      { ...waitOptions, description },
    );
  }

  async startProfile(profileId: number): Promise<void> {
    if (!Number.isSafeInteger(profileId) || profileId < 0 || profileId > 0xffff_ffff) {
      throw new Error("profile ID must be an unsigned 32-bit integer");
    }
    await this.#page.evaluate((id) => globalThis.__VOXELS__!.profile(id), profileId);
  }

  async setSpectator(active: boolean): Promise<readonly number[]> {
    const actual = await this.#page.evaluate(
      (requested) => globalThis.__VOXELS__!.spectator(requested),
      active,
    );
    if (actual !== active) {
      throw new Error(`engine ${active ? "rejected" : "failed to leave"} spectator mode`);
    }
    return this.waitForSnapshot(
      (snapshot) => snapshotValue(snapshot, "spectatorActive") === Number(active),
      { description: `spectator mode did not become ${active ? "active" : "inactive"}` },
    );
  }

  async setDiagnosticSky(rgb: readonly [number, number, number] | null): Promise<void> {
    if (
      rgb !== null &&
      rgb.some((channel) => !Number.isSafeInteger(channel) || channel < 0 || channel > 0xff)
    ) {
      throw new Error("diagnostic sky channels must be unsigned bytes");
    }
    const accepted = await this.#page.evaluate(
      (color) => globalThis.__VOXELS__!.diagnosticSky(color),
      rgb,
    );
    if (!accepted) throw new Error("engine rejected the diagnostic sky override");
    await this.#page.waitForTimeout(50);
  }

  async submitPlace(
    x: number,
    y: number,
    z: number,
    materialId: number,
    shape: AutomationEditShape,
  ): Promise<boolean> {
    return this.#page.evaluate(
      ([voxelX, voxelY, voxelZ, material, editShape]) =>
        globalThis.__VOXELS__!.submitPlace(voxelX, voxelY, voxelZ, material, editShape),
      [x, y, z, materialId, shape] as const,
    );
  }

  async submitDig(x: number, y: number, z: number, shape: AutomationEditShape): Promise<boolean> {
    return this.#page.evaluate(
      ([voxelX, voxelY, voxelZ, editShape]) =>
        globalThis.__VOXELS__!.submitDig(voxelX, voxelY, voxelZ, editShape),
      [x, y, z, shape] as const,
    );
  }

  async inventory(): Promise<readonly number[]> {
    return this.#page.evaluate(() => globalThis.__VOXELS__!.inventory());
  }

  async playerSession(): Promise<BrowserPlayerSession> {
    const player = await this.#page.evaluate(() => globalThis.__VOXELS__!.player);
    if (
      typeof player.browserUserId !== "string" ||
      typeof player.playerId !== "string" ||
      typeof player.playerName !== "string"
    ) {
      throw new Error("engine returned an invalid browser player session");
    }
    return Object.freeze(player);
  }

  async surfaceEditState(stride: number, x: number, z: number): Promise<SurfaceEditState> {
    const values = await this.#page.evaluate(
      ([surfaceStride, voxelX, voxelZ]) =>
        globalThis.__VOXELS__!.surfaceEditState(surfaceStride, voxelX, voxelZ),
      [stride, x, z] as const,
    );
    if (values.length !== 10) {
      throw new Error(`surface edit state returned ${values.length} values instead of 10`);
    }
    const [
      tileX,
      tileZ,
      requiredServerRevision,
      acceptedServerRevision,
      resident,
      dirty,
      fingerprintLow,
      fingerprintHigh,
      quadCount,
      activationMask,
    ] = values;
    if (
      tileX === undefined ||
      tileZ === undefined ||
      requiredServerRevision === undefined ||
      acceptedServerRevision === undefined ||
      resident === undefined ||
      dirty === undefined ||
      fingerprintLow === undefined ||
      fingerprintHigh === undefined ||
      quadCount === undefined ||
      activationMask === undefined
    ) {
      throw new Error("surface edit state is incomplete");
    }
    return Object.freeze({
      tileX,
      tileZ,
      requiredServerRevision,
      acceptedServerRevision,
      resident: resident === 1,
      dirty: dirty === 1,
      fingerprint: (BigInt(fingerprintHigh) << 32n) | BigInt(fingerprintLow),
      quadCount,
      activationMask,
    });
  }
}

export function gpuSampleStart(snapshot: readonly number[]): number {
  const sampleCount = snapshot[SNAPSHOT.sampleCount];
  if (sampleCount === undefined) throw new Error("snapshot omitted its frame sample count");
  return SNAPSHOT.droppedSamples + 1 + sampleCount * FRAME_SAMPLE_WIDTH;
}

export function snapshotValue(snapshot: readonly number[], field: SnapshotField): number {
  const value = snapshot[SNAPSHOT[field]];
  if (value === undefined) throw new Error(`snapshot omitted ${field}`);
  return value;
}
