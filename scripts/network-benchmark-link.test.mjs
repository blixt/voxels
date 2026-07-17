import { describe, expect, it } from "vite-plus/test";
import { serializationMilliseconds, testInternals } from "./network-benchmark-link.mjs";

function maskedFrame(
  payload,
  { opcode = 0x2, final = true, mask = Buffer.from([0x12, 0x34, 0x56, 0x78]) } = {},
) {
  const length = payload.length;
  if (length >= 126) throw new Error("test helper only supports short frames");
  const frame = Buffer.alloc(2 + mask.length + length);
  frame[0] = (final ? 0x80 : 0) | opcode;
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

  it("shares one serialization budget across concurrent connections", () => {
    const clock = new testInternals.SerializationClock();
    const settings = {
      enqueuedMs: 1_000,
      oneWayLatencyMs: 20,
      megabitsPerSecond: 10,
    };
    const first = clock.reserve(1_250_000, settings);
    const second = clock.reserve(1_250_000, settings);
    expect(first).toBe(2_020);
    expect(second).toBe(3_020);
  });

  it("parses masked WebSocket binary frames across arbitrary TCP boundaries", () => {
    const messages = [];
    const parser = new testInternals.WebSocketFrameParser((opcode, payload, frameBytes) => {
      messages.push({ opcode, payload: payload.toString("utf8"), frameBytes });
    });
    const frame = maskedFrame(Buffer.from("VXWP fixture"));
    parser.push(frame.subarray(0, 3));
    parser.push(frame.subarray(3, 8));
    parser.push(frame.subarray(8));
    expect(messages).toEqual([{ opcode: 2, payload: "VXWP fixture", frameBytes: frame.length }]);
  });

  it("attributes every frame byte in a fragmented WebSocket message", () => {
    const messages = [];
    const parser = new testInternals.WebSocketFrameParser(
      (opcode, payload, frameBytes, frameCount) => {
        messages.push({ opcode, payload: payload.toString("utf8"), frameBytes, frameCount });
      },
    );
    const first = maskedFrame(Buffer.from("VXWP "), { opcode: 0x2, final: false });
    const second = maskedFrame(Buffer.from("fixture"), { opcode: 0x0 });
    parser.push(Buffer.concat([first, second]));

    expect(messages).toEqual([
      {
        opcode: 2,
        payload: "VXWP fixture",
        frameBytes: first.length + second.length,
        frameCount: 2,
      },
    ]);
  });
});
