import type { Page } from "playwright";
import {
  FRAME_SAMPLE_WIDTH,
  GPU_SAMPLE_WIDTH,
  SNAPSHOT,
  SNAPSHOT_SCHEMA_VERSION,
  assertAutomationContract,
  assertSnapshotSchema,
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

  async look(deltaX: number, deltaY: number): Promise<void> {
    await this.#page.evaluate(([x, y]) => globalThis.__VOXELS__!.look(x, y), [
      deltaX,
      deltaY,
    ] as const);
  }

  async startProfile(profileId: number): Promise<void> {
    if (!Number.isSafeInteger(profileId) || profileId < 0 || profileId > 0xffff_ffff) {
      throw new Error("profile ID must be an unsigned 32-bit integer");
    }
    await this.#page.evaluate((id) => globalThis.__VOXELS__!.profile(id), profileId);
  }

  async submitEdit(x: number, y: number, z: number, materialId: number): Promise<boolean> {
    return this.#page.evaluate(
      ([voxelX, voxelY, voxelZ, material]) =>
        globalThis.__VOXELS__!.submitEdit(voxelX, voxelY, voxelZ, material),
      [x, y, z, materialId] as const,
    );
  }

  async submitDig(x: number, y: number, z: number): Promise<boolean> {
    return this.#page.evaluate(
      ([voxelX, voxelY, voxelZ]) => globalThis.__VOXELS__!.submitDig(voxelX, voxelY, voxelZ),
      [x, y, z] as const,
    );
  }

  async inventory(): Promise<readonly number[]> {
    return this.#page.evaluate(() => globalThis.__VOXELS__!.inventory());
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
