import { describe, expect, it } from "vite-plus/test";
import {
  PressedKeys,
  WheelAccumulator,
  keyCode,
  requestPointerLockSafely,
  shouldCancelInputForVisibility,
} from "./input.ts";

describe("browser key state", () => {
  it("keeps aliased Shift input active until both physical keys are released", () => {
    const keys = new PressedKeys();

    expect(keys.keyDown("ShiftLeft")).toBe(6);
    expect(keys.keyDown("ShiftRight")).toBe(6);
    expect(keys.keyUp("ShiftLeft")).toBe(0);
    expect(keys.keyUp("ShiftRight")).toBe(6);
  });

  it("resets physical aliases when browser input is cancelled", () => {
    const keys = new PressedKeys();
    keys.keyDown("ShiftLeft");
    keys.clear();

    expect(keys.keyUp("ShiftRight")).toBe(6);
  });

  it("cancels held input when the page becomes hidden", () => {
    expect(shouldCancelInputForVisibility("hidden")).toBe(true);
    expect(shouldCancelInputForVisibility("visible")).toBe(false);
  });

  it("preserves repeatable key-down events and ignores unknown keys", () => {
    const keys = new PressedKeys();

    expect(keys.keyDown("F3")).toBe(8);
    expect(keys.keyDown("F3")).toBe(8);
    expect(keys.keyDown("KeyQ")).toBe(7);
    expect(keys.keyDown("Digit1")).toBe(9);
    expect(keys.keyDown("Digit0")).toBe(18);
    expect(keyCode("Escape")).toBe(0);
    expect(keys.keyUp("Escape")).toBe(0);
  });

  it("handles rejected pointer lock requests", async () => {
    const failure = new Error("pointer lock denied");
    let reported: unknown;

    await requestPointerLockSafely(
      () => Promise.reject(failure),
      (error) => {
        reported = error;
      },
    );

    expect(reported).toBe(failure);
  });
});

describe("inventory wheel normalization", () => {
  it("accumulates high-resolution trackpad deltas before changing selection", () => {
    const wheel = new WheelAccumulator();
    expect(wheel.consume(20, 0, 800)).toEqual([]);
    expect(wheel.consume(30, 0, 800)).toEqual([]);
    expect(wheel.consume(50, 0, 800)).toEqual([1]);
  });

  it("normalizes line and page wheels and bounds one event", () => {
    const wheel = new WheelAccumulator();
    expect(wheel.consume(-3, 1, 800)).toEqual([-1]);
    expect(wheel.consume(1, 2, 800)).toEqual([1, 1, 1, 1]);
  });

  it("drops stale momentum when the wheel reverses", () => {
    const wheel = new WheelAccumulator();
    expect(wheel.consume(70, 0, 800)).toEqual([]);
    expect(wheel.consume(-20, 0, 800)).toEqual([]);
    expect(wheel.consume(-80, 0, 800)).toEqual([-1]);
    wheel.clear();
    expect(wheel.consume(99, 0, 800)).toEqual([]);
  });
});
