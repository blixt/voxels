import { expect, test } from "bun:test";

import { FrameTimingBuckets } from "../src/engine/frame-timing-buckets.ts";

test("frame timing buckets aggregate normal frames into 8Hz windows", () => {
  const timings = new FrameTimingBuckets(125, 50, 8, 1000 / 60);

  for (let timestamp = 0; timestamp <= 270; timestamp += 16) {
    timings.record(timestamp);
  }

  const snapshot = timings.snapshot();

  expect(snapshot.bucketMs).toBe(125);
  expect(snapshot.recent.length).toBeGreaterThanOrEqual(2);
  expect(snapshot.recentHitchCount).toBe(0);
  expect(snapshot.worstRecentFrameMs).toBeLessThan(20);
});

test("frame timing buckets preserve hitches and dropped-frame estimates", () => {
  const timings = new FrameTimingBuckets(125, 50, 8, 1000 / 60);

  timings.record(0);
  timings.record(16);
  timings.record(432);
  timings.record(448);

  const snapshot = timings.snapshot();

  expect(snapshot.recentHitchCount).toBe(1);
  expect(snapshot.worstRecentFrameMs).toBe(416);
  expect(snapshot.recentDroppedFrameEstimate).toBeGreaterThanOrEqual(24);
});
