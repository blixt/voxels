import { describe, expect, it } from "vite-plus/test";
import { PassThrough } from "node:stream";
import { reserveEphemeralPort } from "./browser.ts";
import { createShapedLink, serializationMilliseconds, testInternals } from "./network.ts";

function maskedFrame(
  payload: Buffer,
  {
    opcode = 0x2,
    final = true,
    mask = Buffer.from([0x12, 0x34, 0x56, 0x78]),
  }: {
    readonly opcode?: number;
    readonly final?: boolean;
    readonly mask?: Buffer;
  } = {},
): Buffer {
  const length = payload.length;
  if (length >= 126) throw new Error("test helper only supports short frames");
  const frame = Buffer.alloc(2 + mask.length + length);
  frame[0] = (final ? 0x80 : 0) | opcode;
  frame[1] = 0x80 | length;
  mask.copy(frame, 2);
  for (let index = 0; index < length; index += 1) {
    const payloadByte = payload[index];
    const maskByte = mask[index & 3];
    if (payloadByte === undefined || maskByte === undefined) {
      throw new Error("test frame payload or mask is incomplete");
    }
    frame[6 + index] = payloadByte ^ maskByte;
  }
  return frame;
}

function vxwpFrame(kind: number, bytes: number, sequence: number): Buffer {
  const frame = Buffer.alloc(bytes);
  frame.write("VXWP");
  frame.writeUInt16LE(kind, 6);
  frame.writeUInt32LE(sequence, 24);
  return frame;
}

function vxwpError(message: string): Buffer {
  const text = Buffer.from(message);
  const frame = Buffer.alloc(26 + text.length);
  frame.write("VXWP");
  frame.writeUInt16LE(6, 6);
  frame.writeUInt16LE(text.length, 24);
  text.copy(frame, 26);
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
    const messages: { opcode: number; payload: string; frameBytes: number }[] = [];
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
    const messages: {
      opcode: number;
      payload: string;
      frameBytes: number;
      frameCount: number;
    }[] = [];
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

  it("decodes world-product priority from the typed VXWP request payload", () => {
    const request = Buffer.alloc(25);
    request.write("VXWP");
    request[24] = 1;
    expect(testInternals.worldProductPriorityName(request)).toBe("collision_critical");
    request[24] = 5;
    expect(testInternals.worldProductPriorityName(request)).toBe("prefetch");
    request[24] = 0;
    expect(testInternals.worldProductPriorityName(request)).toBeNull();
  });

  it("measures ping to pong inside the proxy without client scheduling time", () => {
    let now = 100;
    const stats = testInternals.blankStats();
    const inspector = new testInternals.ConnectionInspector({ current: stats }, () => now);
    inspector.path = "/v26/presence";
    inspector.onMessage("upstream", 0x2, vxwpFrame(13, 40, 7), 40, 1);
    now = 112.25;
    inspector.onMessage("downstream", 0x2, vxwpFrame(14, 56, 7), 56, 1);

    expect(testInternals.clonedStats(stats).presenceProxyRoundTrip).toEqual({
      samples: 1,
      p50Ms: 13,
      p95Ms: 13,
      p99Ms: 13,
      maxMs: 12.25,
      meanMs: 12.25,
    });

    inspector.onMessage("upstream", 0x2, vxwpFrame(13, 40, 8), 40, 1);
    inspector.resetRoundTripState();
    now = 140;
    inspector.onMessage("downstream", 0x2, vxwpFrame(14, 56, 8), 56, 1);
    expect(testInternals.clonedStats(stats).presenceProxyRoundTrip.samples).toBe(1);
  });

  it("retains control-error reasons instead of reporting opaque error-frame counts", () => {
    const stats = testInternals.blankStats();
    const inspector = new testInternals.ConnectionInspector({ current: stats });
    inspector.path = "/v26/world";
    inspector.onMessage("downstream", 0x2, vxwpError("world generator is busy"), 49, 1);
    inspector.onMessage("downstream", 0x2, vxwpError("world generator is busy"), 49, 1);

    expect(testInternals.clonedStats(stats).messages["downstream:error"]?.controlErrors).toEqual({
      "world generator is busy": 2,
    });
  });

  it("delivers artificially delayed bytes before forwarding TCP EOF", async () => {
    const source = new PassThrough();
    const destination = new PassThrough();
    const received: Buffer[] = [];
    destination.on("data", (bytes) => received.push(Buffer.from(bytes)));
    const ended = new Promise((resolve) => destination.once("end", resolve));
    const inspector = {
      observe: () => {},
      observeQueue: () => {},
      observeBackpressure: () => {},
    };
    testInternals.shapeDirection(source, destination, inspector, "downstream", {
      oneWayLatencyMs: 10,
      megabitsPerSecond: 1_000,
      quantumBytes: 3,
      maxQueuedBytes: 1_024,
      clock: new testInternals.SerializationClock(),
    });

    source.end(Buffer.from("final VXWP error"));
    await ended;

    expect(Buffer.concat(received).toString("utf8")).toBe("final VXWP error");
  });

  it("can be closed explicitly before scenario cleanup", async () => {
    const [listenPort, targetPort] = await Promise.all([
      reserveEphemeralPort(),
      reserveEphemeralPort(),
    ]);
    const link = await createShapedLink({
      listenPort,
      targetPort,
      profile: {
        name: "close-test",
        oneWayLatencyMs: 0,
        upstreamMegabitsPerSecond: 1_000,
        downstreamMegabitsPerSecond: 1_000,
      },
    });

    await link.close();
    await expect(link.close()).resolves.toBeUndefined();
  });
});
