import { describe, expect, it } from "vite-plus/test";
import {
  PressedKeys,
  WheelAccumulator,
  inputAllowedWhileLoading,
  keyCode,
  requestPointerLockSafely,
  shouldCancelInputForVisibility,
} from "./input.ts";
import { INPUT_CANCEL, INPUT_KEY_DOWN, INPUT_POINTER_DOWN, type InputSample } from "./protocol.ts";

describe("browser key state", () => {
  const sample = (kind: number, code: number): InputSample => ({
    kind,
    code,
    buttons: 0,
    x: 0,
    y: 0,
    dx: 0,
    dy: 0,
    flags: 0,
  });

  it("allows only cancellation and F2 evidence capture while loading", () => {
    expect(inputAllowedWhileLoading(sample(INPUT_CANCEL, 0))).toBe(true);
    expect(inputAllowedWhileLoading(sample(INPUT_KEY_DOWN, keyCode("F2")))).toBe(true);
    expect(inputAllowedWhileLoading({ ...sample(INPUT_KEY_DOWN, keyCode("F2")), flags: 1 })).toBe(
      false,
    );
    expect(inputAllowedWhileLoading(sample(INPUT_KEY_DOWN, keyCode("KeyW")))).toBe(false);
    expect(inputAllowedWhileLoading(sample(INPUT_POINTER_DOWN, 0))).toBe(false);
  });

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
    expect(keys.keyDown("F2")).toBe(19);
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
