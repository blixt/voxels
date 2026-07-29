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

  async applyReproduction(metadata: string): Promise<readonly number[]> {
    if (metadata.length === 0) throw new Error("reproduction metadata must not be empty");
    await this.#page.evaluate(
      (reproduction) => globalThis.__VOXELS__!.applyReproduction(reproduction),
      metadata,
    );
    const expected = JSON.parse(metadata) as {
      camera?: { eyeMetres?: number[]; yawRadians?: number; pitchRadians?: number };
    };
    const eye = expected.camera?.eyeMetres;
    if (
      eye?.length !== 3 ||
      !eye.every(Number.isFinite) ||
      !Number.isFinite(expected.camera?.yawRadians) ||
      !Number.isFinite(expected.camera?.pitchRadians)
    ) {
      throw new Error("reproduction metadata omitted the exact camera pose");
    }
    return this.waitForSnapshot(
      (snapshot) =>
        Math.hypot(
          snapshotValue(snapshot, "cameraX") - eye[0]!,
          snapshotValue(snapshot, "cameraY") - eye[1]!,
          snapshotValue(snapshot, "cameraZ") - eye[2]!,
        ) <= 0.000_01 &&
        Math.abs(snapshotValue(snapshot, "yaw") - expected.camera!.yawRadians!) <= 0.000_01 &&
        Math.abs(snapshotValue(snapshot, "pitch") - expected.camera!.pitchRadians!) <= 0.000_01,
      { description: "engine did not freeze the exact reproduction camera" },
    );
  }

  async clearReproduction(): Promise<void> {
    await this.#page.evaluate(() => globalThis.__VOXELS__!.clearReproduction());
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

  async waitForFrameAfter(
    frameSequence: number,
    options: SnapshotWaitOptions = {},
  ): Promise<readonly number[]> {
    if (!Number.isSafeInteger(frameSequence) || frameSequence < 0) {
      throw new Error("frame sequence must be a non-negative integer");
    }
    return this.waitForSnapshot(
      (snapshot) => snapshotValue(snapshot, "frameSequence") !== frameSequence,
      { ...options, description: options.description ?? "renderer did not advance a frame" },
    );
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
    if (this.#contract === undefined) {
      await this.#page.waitForTimeout(50);
    } else {
      // The snapshot request is ordered after the worker command and therefore acknowledges that
      // the state was applied. Wait one complete frame beyond that acknowledgement so a browser
      // screenshot cannot capture the canvas commit immediately preceding the command.
      const acknowledged = await this.snapshot();
      await this.waitForFrameAfter(snapshotValue(acknowledged, "frameSequence"), {
        description: "diagnostic sky override was not presented",
      });
    }
  }

  async setGeometrySourceDebug(enabled: boolean): Promise<void> {
    const accepted = await this.#page.evaluate(
      (active) => globalThis.__VOXELS__!.geometrySourceDebug(active),
      enabled,
    );
    if (!accepted) {
      throw new Error("engine rejected the geometry-source diagnostic");
    }
    if (this.#contract === undefined) {
      await this.#page.waitForTimeout(50);
    } else {
      const acknowledged = await this.snapshot();
      await this.waitForFrameAfter(snapshotValue(acknowledged, "frameSequence"), {
        description: "geometry-source diagnostic was not presented",
      });
    }
  }

  async setMaterialDetail(enabled: boolean): Promise<readonly number[]> {
    const accepted = await this.#page.evaluate(
      (active) => globalThis.__VOXELS__!.materialDetail(active),
      enabled,
    );
    if (!accepted) throw new Error("engine rejected the material-detail override");
    return this.waitForSnapshot(
      (snapshot) => snapshotValue(snapshot, "materialDetail") === Number(enabled),
      { description: `material detail did not become ${enabled ? "enabled" : "disabled"}` },
    );
  }

  async exactVolumePresented(voxel: readonly [number, number, number]): Promise<boolean> {
    if (
      voxel.some(
        (value) => !Number.isSafeInteger(value) || value < -0x8000_0000 || value > 0x7fff_ffff,
      )
    ) {
      throw new Error("exact-volume voxel coordinates must be signed 32-bit integers");
    }
    return this.#page.evaluate(
      ([x, y, z]) => globalThis.__VOXELS__!.exactVolumePresented(x, y, z),
      voxel,
    );
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
}

export function snapshotValue(snapshot: readonly number[], field: SnapshotField): number {
  const value = snapshot[SNAPSHOT[field]];
  if (value === undefined) throw new Error(`snapshot omitted ${field}`);
  return value;
}
