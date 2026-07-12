import { describe, expect, it } from "vite-plus/test";
import { INPUT_POINTER_MOVE, INPUT_RECORD_BYTES, packInput } from "./protocol.ts";

describe("packed input protocol", () => {
  it("matches the Rust InputRecord layout", () => {
    const buffer = packInput([
      {
        kind: INPUT_POINTER_MOVE,
        code: 2,
        buttons: 0x1234,
        x: 10.5,
        y: 20.25,
        dx: -3.5,
        dy: 4.75,
        flags: 0x89abcdef,
      },
    ]);
    const view = new DataView(buffer);
    expect(buffer.byteLength).toBe(INPUT_RECORD_BYTES);
    expect(view.getUint8(0)).toBe(INPUT_POINTER_MOVE);
    expect(view.getUint8(1)).toBe(2);
    expect(view.getUint16(2, true)).toBe(0x1234);
    expect(view.getFloat32(4, true)).toBe(10.5);
    expect(view.getFloat32(8, true)).toBe(20.25);
    expect(view.getFloat32(12, true)).toBe(-3.5);
    expect(view.getFloat32(16, true)).toBe(4.75);
    expect(view.getUint32(20, true)).toBe(0x89abcdef);
  });
});
