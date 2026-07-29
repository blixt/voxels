import { type EngineClient, snapshotValue } from "./engine.ts";

export const PLAYER_PRESENTATION_TRACE_LIMIT = 240;

export type PlayerPresentationPhase =
  | "startup"
  | "pedestal-step"
  | "pedestal-settle"
  | "travel"
  | "travel-settle"
  | "dig"
  | "dig-settle";

export interface PlayerPresentationTraceFrame {
  readonly phase: PlayerPresentationPhase;
  readonly frameSequence: number;
  readonly camera: readonly [number, number, number];
  readonly terrainReady: boolean;
  readonly renderMode: number;
  readonly published: {
    readonly pages: number;
    readonly exactPages: number;
    readonly minimumLevel: number;
    readonly maximumLevel: number;
    readonly ownerlessRoots: number;
    readonly exactLodDiscontinuities: number;
    readonly cutFingerprint: string;
  };
  readonly gpu: {
    readonly candidateMatchesCpu: boolean;
    readonly encodingOverflowFlags: number;
    readonly encodedPages: number;
    readonly ownerlessRoots: number;
    readonly presentedBankGeneration: string;
    readonly presentedBankFingerprint: string;
    readonly presentedBankMatchesCut: boolean;
  };
  readonly exactCore: {
    readonly complete: boolean;
    readonly requiredLeaves: number;
    readonly currentCoverage: number;
  };
  readonly desiredEnvelope: {
    readonly complete: boolean;
    readonly fingerprint: string;
    readonly safetyLeaves: number;
    readonly horizonRoots: number;
    readonly locus: readonly [number, number, number, number];
  };
  readonly committedEnvelope: {
    readonly fingerprint: string;
    readonly safetyLeaves: number;
    readonly safetyCoverage: number;
    readonly horizonRoots: number;
    readonly horizonCoverage: number;
    readonly locus: readonly [number, number, number, number];
  };
  readonly presentationTarget: readonly [number, number, number];
  readonly presentationGate: {
    readonly active: boolean;
    readonly blockedSteps: string;
    readonly blockedFrames: string;
  };
}

export interface PlayerPresentationViolation {
  readonly phase: PlayerPresentationPhase;
  readonly frameSequence: number;
  readonly reasons: readonly string[];
  readonly trace: readonly PlayerPresentationTraceFrame[];
}

export interface PresentationFrameWaitOptions {
  readonly timeoutMs?: number;
  readonly description?: string;
}

export interface PlayerPresentationRecorderOptions {
  readonly initialPhase?: PlayerPresentationPhase;
  readonly traceLimit?: number;
  readonly frameTimeoutMs?: number;
  readonly onFrame?: (frame: PlayerPresentationTraceFrame) => void;
  readonly onViolation?: (violation: PlayerPresentationViolation) => Promise<void>;
}

interface FrameWaiter {
  readonly afterSequence: number;
  readonly resolve: (snapshot: readonly number[]) => void;
  readonly reject: (error: Error) => void;
  readonly timer: ReturnType<typeof setTimeout>;
}

function hexadecimalPart(value: number, digits: number): string {
  if (!Number.isSafeInteger(value) || value < 0) return `invalid(${String(value)})`;
  return value.toString(16).padStart(digits, "0");
}

function identity48(low24: number, high24: number): string {
  return `${hexadecimalPart(high24, 6)}${hexadecimalPart(low24, 6)}`;
}

function identity64(low24: number, mid24: number, high16: number): string {
  return `${hexadecimalPart(high16, 4)}${hexadecimalPart(mid24, 6)}${hexadecimalPart(low24, 6)}`;
}

function isZeroIdentity(value: string): boolean {
  return /^0+$/u.test(value);
}

function traceFrame(
  snapshot: readonly number[],
  phase: PlayerPresentationPhase,
): PlayerPresentationTraceFrame {
  return {
    phase,
    frameSequence: snapshotValue(snapshot, "frameSequence"),
    camera: [
      snapshotValue(snapshot, "cameraX"),
      snapshotValue(snapshot, "cameraY"),
      snapshotValue(snapshot, "cameraZ"),
    ],
    terrainReady: snapshotValue(snapshot, "terrainReady") === 1,
    renderMode: snapshotValue(snapshot, "virtualTerrainMode"),
    published: {
      pages: snapshotValue(snapshot, "virtualTerrainPublishedPages"),
      exactPages: snapshotValue(snapshot, "virtualTerrainPublishedExactPages"),
      minimumLevel: snapshotValue(snapshot, "virtualTerrainPublishedMinimumLevel"),
      maximumLevel: snapshotValue(snapshot, "virtualTerrainPublishedMaximumLevel"),
      ownerlessRoots: snapshotValue(snapshot, "virtualTerrainOwnerlessRoots"),
      exactLodDiscontinuities: snapshotValue(
        snapshot,
        "virtualTerrainPublishedExactLodDiscontinuities",
      ),
      cutFingerprint: identity48(
        snapshotValue(snapshot, "virtualTerrainCutFingerprintLow24"),
        snapshotValue(snapshot, "virtualTerrainCutFingerprintHigh24"),
      ),
    },
    gpu: {
      candidateMatchesCpu: snapshotValue(snapshot, "virtualTerrainGpuMatchesCpuCut") === 1,
      encodingOverflowFlags: snapshotValue(snapshot, "virtualTerrainGpuEncodingOverflowFlags"),
      encodedPages: snapshotValue(snapshot, "virtualTerrainGpuEncodedPages"),
      ownerlessRoots: snapshotValue(snapshot, "virtualTerrainGpuOwnerlessRoots"),
      presentedBankGeneration: identity48(
        snapshotValue(snapshot, "virtualTerrainPresentedSnapshotGenerationLow24"),
        snapshotValue(snapshot, "virtualTerrainPresentedSnapshotGenerationHigh24"),
      ),
      presentedBankFingerprint: identity48(
        snapshotValue(snapshot, "virtualTerrainPresentedSnapshotFingerprintLow24"),
        snapshotValue(snapshot, "virtualTerrainPresentedSnapshotFingerprintHigh24"),
      ),
      presentedBankMatchesCut:
        snapshotValue(snapshot, "virtualTerrainPresentedSnapshotMatchesCut") === 1,
    },
    exactCore: {
      complete: snapshotValue(snapshot, "virtualTerrainExactCoreComplete") === 1,
      requiredLeaves: snapshotValue(snapshot, "virtualTerrainExactCoreRequiredLeaves"),
      currentCoverage: snapshotValue(snapshot, "virtualTerrainExactCoreCurrentCoverage"),
    },
    desiredEnvelope: {
      complete: snapshotValue(snapshot, "virtualTerrainDesiredEnvelopeComplete") === 1,
      fingerprint: identity64(
        snapshotValue(snapshot, "virtualTerrainDesiredEnvelopeFingerprintLow24"),
        snapshotValue(snapshot, "virtualTerrainDesiredEnvelopeFingerprintMid24"),
        snapshotValue(snapshot, "virtualTerrainDesiredEnvelopeFingerprintHigh16"),
      ),
      safetyLeaves: snapshotValue(snapshot, "virtualTerrainDesiredSafetyLeaves"),
      horizonRoots: snapshotValue(snapshot, "virtualTerrainDesiredHorizonRoots"),
      locus: [
        snapshotValue(snapshot, "virtualTerrainDesiredLocusMinimumLeafX"),
        snapshotValue(snapshot, "virtualTerrainDesiredLocusMinimumLeafZ"),
        snapshotValue(snapshot, "virtualTerrainDesiredLocusMaximumLeafExclusiveX"),
        snapshotValue(snapshot, "virtualTerrainDesiredLocusMaximumLeafExclusiveZ"),
      ],
    },
    committedEnvelope: {
      fingerprint: identity64(
        snapshotValue(snapshot, "virtualTerrainCommittedEnvelopeFingerprintLow24"),
        snapshotValue(snapshot, "virtualTerrainCommittedEnvelopeFingerprintMid24"),
        snapshotValue(snapshot, "virtualTerrainCommittedEnvelopeFingerprintHigh16"),
      ),
      safetyLeaves: snapshotValue(snapshot, "virtualTerrainCommittedSafetyLeaves"),
      safetyCoverage: snapshotValue(snapshot, "virtualTerrainCommittedSafetyCoverage"),
      horizonRoots: snapshotValue(snapshot, "virtualTerrainCommittedHorizonRoots"),
      horizonCoverage: snapshotValue(snapshot, "virtualTerrainCommittedHorizonCoverage"),
      locus: [
        snapshotValue(snapshot, "virtualTerrainCommittedLocusMinimumLeafX"),
        snapshotValue(snapshot, "virtualTerrainCommittedLocusMinimumLeafZ"),
        snapshotValue(snapshot, "virtualTerrainCommittedLocusMaximumLeafExclusiveX"),
        snapshotValue(snapshot, "virtualTerrainCommittedLocusMaximumLeafExclusiveZ"),
      ],
    },
    presentationTarget: [
      snapshotValue(snapshot, "presentationTargetX"),
      snapshotValue(snapshot, "presentationTargetY"),
      snapshotValue(snapshot, "presentationTargetZ"),
    ],
    presentationGate: {
      active: snapshotValue(snapshot, "presentationGateActive") === 1,
      blockedSteps: identity64(
        snapshotValue(snapshot, "presentationGateStepsLow24"),
        snapshotValue(snapshot, "presentationGateStepsMid24"),
        snapshotValue(snapshot, "presentationGateStepsHigh16"),
      ),
      blockedFrames: identity64(
        snapshotValue(snapshot, "presentationGateFramesLow24"),
        snapshotValue(snapshot, "presentationGateFramesMid24"),
        snapshotValue(snapshot, "presentationGateFramesHigh16"),
      ),
    },
  };
}

function invalidCommittedPresentation(frame: PlayerPresentationTraceFrame): string[] {
  const reasons: string[] = [];
  if (frame.renderMode !== 2) reasons.push(`render mode regressed to ${frame.renderMode}`);
  if (frame.published.pages <= 0) reasons.push("published cut became empty");
  if (frame.published.exactPages <= 0) reasons.push("published cut lost every exact L0 page");
  if (frame.published.minimumLevel !== 0) {
    reasons.push(`published minimum LOD regressed to L${frame.published.minimumLevel}`);
  }
  if (frame.published.ownerlessRoots !== 0) {
    reasons.push(`published cut has ${frame.published.ownerlessRoots} ownerless roots`);
  }
  if (frame.published.exactLodDiscontinuities !== 0) {
    reasons.push(
      `published cut has ${frame.published.exactLodDiscontinuities} skipped-level edges`,
    );
  }
  if (isZeroIdentity(frame.published.cutFingerprint)) {
    reasons.push("published cut fingerprint became zero");
  }
  if (frame.gpu.encodingOverflowFlags !== 0) {
    reasons.push(`GPU encoding overflow flags are ${frame.gpu.encodingOverflowFlags}`);
  }
  if (!frame.gpu.presentedBankMatchesCut) {
    reasons.push("presented GPU bank does not match the published CPU cut");
  }
  if (isZeroIdentity(frame.gpu.presentedBankGeneration)) {
    reasons.push("presented GPU bank generation became zero");
  }
  if (isZeroIdentity(frame.gpu.presentedBankFingerprint)) {
    reasons.push("presented GPU bank fingerprint became zero");
  }
  const committed = frame.committedEnvelope;
  if (isZeroIdentity(committed.fingerprint)) {
    reasons.push("committed presentation envelope fingerprint became zero");
  }
  if (committed.safetyLeaves <= 0 || committed.safetyCoverage !== committed.safetyLeaves) {
    reasons.push(
      `committed exact safety coverage is ${committed.safetyCoverage}/${committed.safetyLeaves}`,
    );
  }
  if (committed.horizonRoots <= 0 || committed.horizonCoverage !== committed.horizonRoots) {
    reasons.push(
      `committed horizon coverage is ${committed.horizonCoverage}/${committed.horizonRoots}`,
    );
  }
  const [minimumX, minimumZ, maximumX, maximumZ] = committed.locus;
  if (maximumX <= minimumX || maximumZ <= minimumZ) {
    reasons.push(`committed presentation locus is empty: ${committed.locus.join(",")}`);
  }
  if (frame.terrainReady) {
    const core = frame.exactCore;
    if (
      !core.complete ||
      core.requiredLeaves <= 0 ||
      core.currentCoverage !== core.requiredLeaves
    ) {
      reasons.push(
        `admitted gameplay position has exact core ${core.currentCoverage}/${core.requiredLeaves}, complete=${core.complete}`,
      );
    }
  }
  return reasons;
}

export class PlayerPresentationInvariantState {
  readonly #traceLimit: number;
  readonly #trace: PlayerPresentationTraceFrame[] = [];
  #lastFrameSequence: number | undefined;
  #committedPresentationSeen = false;
  #firstPlayableFrameSequence: number | undefined;
  #observedFrames = 0;

  constructor(traceLimit = PLAYER_PRESENTATION_TRACE_LIMIT) {
    if (!Number.isSafeInteger(traceLimit) || traceLimit <= 0) {
      throw new Error("presentation trace limit must be a positive safe integer");
    }
    this.#traceLimit = traceLimit;
  }

  get observedFrames(): number {
    return this.#observedFrames;
  }

  get firstPlayableFrameSequence(): number | undefined {
    return this.#firstPlayableFrameSequence;
  }

  trace(): readonly PlayerPresentationTraceFrame[] {
    return this.#trace.slice();
  }

  observe(
    snapshot: readonly number[],
    phase: PlayerPresentationPhase,
  ): PlayerPresentationViolation | undefined {
    const frame = traceFrame(snapshot, phase);
    if (frame.frameSequence === this.#lastFrameSequence) return undefined;
    this.#lastFrameSequence = frame.frameSequence;
    this.#observedFrames += 1;
    this.#trace.push(frame);
    if (this.#trace.length > this.#traceLimit) this.#trace.shift();

    if (frame.renderMode === 2 && !this.#committedPresentationSeen) {
      this.#committedPresentationSeen = true;
      this.#firstPlayableFrameSequence = frame.frameSequence;
    }
    if (!this.#committedPresentationSeen) return undefined;

    const reasons = invalidCommittedPresentation(frame);
    if (reasons.length === 0) return undefined;
    return {
      phase,
      frameSequence: frame.frameSequence,
      reasons,
      trace: this.trace(),
    };
  }
}

export class PlayerPresentationViolationError extends Error {
  readonly violation: PlayerPresentationViolation;

  constructor(violation: PlayerPresentationViolation, evidenceError?: unknown) {
    const evidence =
      evidenceError === undefined
        ? ""
        : `; preserving failure evidence failed: ${
            evidenceError instanceof Error ? evidenceError.message : "unknown non-Error failure"
          }`;
    super(
      `player presentation invariant failed during ${violation.phase} frame ${violation.frameSequence}: ${violation.reasons.join("; ")}${evidence}`,
    );
    this.name = "PlayerPresentationViolationError";
    this.violation = violation;
  }
}

export class PlayerPresentationRecorder {
  readonly #engine: EngineClient;
  readonly #state: PlayerPresentationInvariantState;
  readonly #frameTimeoutMs: number;
  readonly #onFrame: ((frame: PlayerPresentationTraceFrame) => void) | undefined;
  readonly #onViolation: ((violation: PlayerPresentationViolation) => Promise<void>) | undefined;
  readonly #waiters = new Set<FrameWaiter>();
  readonly #failureSignal: Promise<Error>;
  readonly #stopSignal: Promise<void>;
  #resolveFailure!: (error: Error) => void;
  #resolveStop!: () => void;
  #phase: PlayerPresentationPhase;
  #latestSnapshot: readonly number[] | undefined;
  #latestFrame: PlayerPresentationTraceFrame | undefined;
  #failure: Error | undefined;
  #pump: Promise<void> | undefined;
  #stopped = false;

  constructor(engine: EngineClient, options: PlayerPresentationRecorderOptions = {}) {
    this.#engine = engine;
    this.#state = new PlayerPresentationInvariantState(options.traceLimit);
    this.#phase = options.initialPhase ?? "startup";
    this.#frameTimeoutMs = options.frameTimeoutMs ?? 15_000;
    this.#onFrame = options.onFrame;
    this.#onViolation = options.onViolation;
    this.#failureSignal = new Promise((resolve) => {
      this.#resolveFailure = resolve;
    });
    this.#stopSignal = new Promise((resolve) => {
      this.#resolveStop = resolve;
    });
  }

  get observedFrames(): number {
    return this.#state.observedFrames;
  }

  get firstPlayableFrameSequence(): number | undefined {
    return this.#state.firstPlayableFrameSequence;
  }

  get latestSnapshot(): readonly number[] {
    if (this.#latestSnapshot === undefined) {
      throw new Error("player presentation recorder has not started");
    }
    return this.#latestSnapshot;
  }

  get latestFrame(): PlayerPresentationTraceFrame {
    if (this.#latestFrame === undefined) {
      throw new Error("player presentation recorder has not started");
    }
    return this.#latestFrame;
  }

  trace(): readonly PlayerPresentationTraceFrame[] {
    return this.#state.trace();
  }

  setPhase(phase: PlayerPresentationPhase): void {
    this.throwIfFailed();
    this.#phase = phase;
  }

  async start(): Promise<readonly number[]> {
    if (this.#pump !== undefined) throw new Error("player presentation recorder already started");
    const initial = await this.#engine.snapshot();
    await this.#accept(initial);
    this.throwIfFailed();
    this.#pump = this.#pumpFrames();
    return initial;
  }

  async guard<T>(operation: Promise<T>): Promise<T> {
    this.throwIfFailed();
    const outcome = await Promise.race([
      operation.then((value) => ({ kind: "value" as const, value })),
      this.#failureSignal.then((error) => ({ kind: "failure" as const, error })),
    ]);
    if (outcome.kind === "failure") throw outcome.error;
    return outcome.value;
  }

  async waitFor(
    predicate: (snapshot: readonly number[]) => boolean,
    {
      timeoutMs = 5_000,
      description = "recorded player presentation did not reach the requested state",
    }: PresentationFrameWaitOptions = {},
  ): Promise<readonly number[]> {
    const deadline = performance.now() + timeoutMs;
    let current = this.latestSnapshot;
    while (!predicate(current)) {
      const remaining = deadline - performance.now();
      if (remaining <= 0) throw new Error(`${description}: ${JSON.stringify(this.latestFrame)}`);
      current = await this.waitForFrameAfter(snapshotValue(current, "frameSequence"), {
        timeoutMs: remaining,
        description,
      });
    }
    return current;
  }

  waitForFrameAfter(
    frameSequence: number,
    {
      timeoutMs = 5_000,
      description = "recorded renderer did not advance a frame",
    }: PresentationFrameWaitOptions = {},
  ): Promise<readonly number[]> {
    this.throwIfFailed();
    if (!Number.isSafeInteger(frameSequence) || frameSequence < 0) {
      throw new Error("frame sequence must be a non-negative safe integer");
    }
    if (snapshotValue(this.latestSnapshot, "frameSequence") !== frameSequence) {
      return Promise.resolve(this.latestSnapshot);
    }
    return new Promise((resolve, reject) => {
      const waiter: FrameWaiter = {
        afterSequence: frameSequence,
        resolve,
        reject,
        timer: setTimeout(() => {
          this.#waiters.delete(waiter);
          reject(new Error(`${description}: ${JSON.stringify(this.latestFrame)}`));
        }, timeoutMs),
      };
      this.#waiters.add(waiter);
    });
  }

  throwIfFailed(): void {
    if (this.#failure !== undefined) throw this.#failure;
  }

  async stop(): Promise<void> {
    if (this.#stopped) {
      if (this.#pump !== undefined) await this.#pump;
      this.throwIfFailed();
      return;
    }
    this.#stopped = true;
    this.#resolveStop();
    this.#rejectWaiters(new Error("player presentation recorder stopped"));
    if (this.#pump !== undefined) await this.#pump;
    this.throwIfFailed();
  }

  async #pumpFrames(): Promise<void> {
    try {
      while (!this.#stopped && this.#failure === undefined) {
        const sequence = snapshotValue(this.latestSnapshot, "frameSequence");
        const next = await Promise.race([
          this.#engine.waitForFrameAfter(sequence, {
            timeoutMs: this.#frameTimeoutMs,
            intervalMs: 0,
            description: "continuous player presentation recorder stopped receiving frames",
          }),
          this.#stopSignal.then(() => undefined),
        ]);
        if (next === undefined) return;
        await this.#accept(next);
      }
    } catch (error) {
      if (!this.#stopped) this.#fail(error instanceof Error ? error : new Error(String(error)));
    }
  }

  async #accept(snapshot: readonly number[]): Promise<void> {
    this.#latestSnapshot = snapshot;
    const violation = this.#state.observe(snapshot, this.#phase);
    this.#latestFrame = this.#state.trace().at(-1);
    if (this.#latestFrame !== undefined) this.#onFrame?.(this.#latestFrame);
    if (violation !== undefined) {
      let evidenceError: unknown;
      try {
        await this.#onViolation?.(violation);
      } catch (error) {
        evidenceError = error;
      }
      this.#fail(new PlayerPresentationViolationError(violation, evidenceError));
      return;
    }
    const sequence = snapshotValue(snapshot, "frameSequence");
    for (const waiter of this.#waiters) {
      if (sequence === waiter.afterSequence) continue;
      clearTimeout(waiter.timer);
      this.#waiters.delete(waiter);
      waiter.resolve(snapshot);
    }
  }

  #fail(error: Error): void {
    if (this.#failure !== undefined) return;
    this.#failure = error;
    this.#resolveFailure(error);
    this.#rejectWaiters(error);
  }

  #rejectWaiters(error: Error): void {
    for (const waiter of this.#waiters) {
      clearTimeout(waiter.timer);
      waiter.reject(error);
    }
    this.#waiters.clear();
  }
}
