export interface FrameTimingBucket {
  startMs: number;
  endMs: number;
  frameCount: number;
  totalFrameMs: number;
  maxFrameMs: number;
  stalledMs: number;
  hitchCount: number;
  droppedFrameEstimate: number;
}

export interface FrameTimingSnapshot {
  bucketMs: number;
  hitchThresholdMs: number;
  current: FrameTimingBucket;
  recent: readonly FrameTimingBucket[];
  worstRecentFrameMs: number;
  recentStalledMs: number;
  recentHitchCount: number;
  recentDroppedFrameEstimate: number;
}

export class FrameTimingBuckets {
  private readonly buckets: FrameTimingBucket[] = [];
  private current: FrameTimingBucket | null = null;
  private lastTimestampMs: number | null = null;

  constructor(
    readonly bucketMs = 125,
    readonly hitchThresholdMs = 50,
    readonly maxBuckets = 80,
    readonly targetFrameMs = 1000 / 60,
  ) {}

  record(timestampMs: number): FrameTimingSnapshot {
    if (!this.current) {
      this.current = createBucket(timestampMs, this.bucketMs);
      this.lastTimestampMs = timestampMs;
      return this.snapshot();
    }

    const previousTimestampMs = this.lastTimestampMs ?? timestampMs;
    const frameMs = Math.max(0, timestampMs - previousTimestampMs);
    this.lastTimestampMs = timestampMs;

    if (frameMs >= this.hitchThresholdMs) {
      this.recordStallSpan(previousTimestampMs, timestampMs);
    }
    this.advanceCurrentTo(timestampMs);

    this.current.frameCount += 1;
    this.current.totalFrameMs += frameMs;
    this.current.maxFrameMs = Math.max(this.current.maxFrameMs, frameMs);
    if (frameMs >= this.hitchThresholdMs) {
      this.current.hitchCount += 1;
    }
    this.current.droppedFrameEstimate += Math.max(0, Math.round(frameMs / this.targetFrameMs) - 1);

    return this.snapshot();
  }

  snapshot(): FrameTimingSnapshot {
    const current = this.current ?? createBucket(0, this.bucketMs);
    let worstRecentFrameMs = current.maxFrameMs;
    let recentStalledMs = current.stalledMs;
    let recentHitchCount = current.hitchCount;
    let recentDroppedFrameEstimate = current.droppedFrameEstimate;
    for (const bucket of this.buckets) {
      worstRecentFrameMs = Math.max(worstRecentFrameMs, bucket.maxFrameMs);
      recentStalledMs += bucket.stalledMs;
      recentHitchCount += bucket.hitchCount;
      recentDroppedFrameEstimate += bucket.droppedFrameEstimate;
    }
    return {
      bucketMs: this.bucketMs,
      hitchThresholdMs: this.hitchThresholdMs,
      current: cloneBucket(current),
      recent: this.buckets.map(cloneBucket),
      worstRecentFrameMs,
      recentStalledMs,
      recentHitchCount,
      recentDroppedFrameEstimate,
    };
  }

  private recordStallSpan(startMs: number, endMs: number): void {
    let cursorMs = startMs;
    while (cursorMs < endMs) {
      this.advanceCurrentTo(cursorMs);
      const bucket = this.current;
      if (!bucket) {
        return;
      }
      const overlapEndMs = Math.min(endMs, bucket.endMs);
      bucket.stalledMs += Math.max(0, overlapEndMs - cursorMs);
      cursorMs = overlapEndMs;
    }
  }

  private advanceCurrentTo(timestampMs: number): void {
    while (this.current && timestampMs >= this.current.endMs) {
      this.buckets.push(this.current);
      if (this.buckets.length > this.maxBuckets) {
        this.buckets.shift();
      }
      this.current = createBucket(this.current.endMs, this.bucketMs);
    }
  }
}

function createBucket(startMs: number, bucketMs: number): FrameTimingBucket {
  return {
    startMs,
    endMs: startMs + bucketMs,
    frameCount: 0,
    totalFrameMs: 0,
    maxFrameMs: 0,
    stalledMs: 0,
    hitchCount: 0,
    droppedFrameEstimate: 0,
  };
}

function cloneBucket(bucket: FrameTimingBucket): FrameTimingBucket {
  return {
    startMs: bucket.startMs,
    endMs: bucket.endMs,
    frameCount: bucket.frameCount,
    totalFrameMs: bucket.totalFrameMs,
    maxFrameMs: bucket.maxFrameMs,
    stalledMs: bucket.stalledMs,
    hitchCount: bucket.hitchCount,
    droppedFrameEstimate: bucket.droppedFrameEstimate,
  };
}
