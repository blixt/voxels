import { describe, expect, it } from "vite-plus/test";
import {
  compressionEstimates,
  serializationMilliseconds,
  testInternals,
} from "./network-benchmark-link.mjs";

function maskedBinaryFrame(payload, mask = Buffer.from([0x12, 0x34, 0x56, 0x78])) {
  const length = payload.length;
  if (length >= 126) throw new Error("test helper only supports short frames");
  const frame = Buffer.alloc(2 + mask.length + length);
  frame[0] = 0x82;
  frame[1] = 0x80 | length;
  mask.copy(frame, 2);
  for (let index = 0; index < length; index += 1) {
    frame[6 + index] = payload[index] ^ mask[index & 3];
  }
  return frame;
}

describe("network benchmark link", () => {
  it("models decimal megabit serialization independently of propagation", () => {
    expect(serializationMilliseconds(1_250_000, 10)).toBe(1_000);
    expect(serializationMilliseconds(6_250_000, 50)).toBe(1_000);
  });

  it("parses masked WebSocket binary frames across arbitrary TCP boundaries", () => {
    const messages = [];
    const parser = new testInternals.WebSocketFrameParser((opcode, payload, frameBytes) => {
      messages.push({ opcode, payload: payload.toString("utf8"), frameBytes });
    });
    const frame = maskedBinaryFrame(Buffer.from("VXWP fixture"));
    parser.push(frame.subarray(0, 3));
    parser.push(frame.subarray(3, 8));
    parser.push(frame.subarray(8));
    expect(messages).toEqual([{ opcode: 2, payload: "VXWP fixture", frameBytes: frame.length }]);
  });

  it("reports independent compression headroom without changing the captured payload", () => {
    const payload = Buffer.alloc(64 * 1024, 7);
    const estimates = compressionEstimates([{ kind: "surface_tile_batch_result", payload }]);
    const result = estimates.surface_tile_batch_result;
    expect(result.frames).toBe(1);
    expect(result.rawBytes).toBe(payload.length);
    expect(result.zstdLevel1Bytes).toBeLessThan(payload.length / 10);
    expect(result.brotliQuality4Bytes).toBeLessThan(payload.length / 10);
    expect(result.deflateLevel1Bytes).toBeLessThan(payload.length / 10);
    expect(payload.every((byte) => byte === 7)).toBe(true);
  });
});
