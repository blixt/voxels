import { describe, expect, it } from "vite-plus/test";
import { PressedKeys, keyCode } from "./input.ts";

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

  it("preserves repeatable key-down events and ignores unknown keys", () => {
    const keys = new PressedKeys();

    expect(keys.keyDown("F3")).toBe(8);
    expect(keys.keyDown("F3")).toBe(8);
    expect(keyCode("Escape")).toBe(0);
    expect(keys.keyUp("Escape")).toBe(0);
  });
});
