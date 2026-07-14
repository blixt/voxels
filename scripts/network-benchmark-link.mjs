import { createServer, connect } from "node:net";

const HTTP_HEADER_END = Buffer.from("\r\n\r\n");
const MAX_HTTP_HEADER_BYTES = 64 * 1024;
const VXWP_MAGIC = Buffer.from("VXWP");
const DEFAULT_QUANTUM_BYTES = 16 * 1024;
const QUEUE_BANDWIDTH_DELAY_PRODUCTS = 2;

export const VXWP_KIND_NAMES = Object.freeze({
  1: "open_world",
  2: "world_opened",
  3: "chunk_batch",
  4: "chunk_batch_result",
  5: "cancel",
  6: "error",
  7: "surface_tile_batch",
  8: "surface_tile_batch_result",
  9: "open_presence",
  10: "presence_opened",
  11: "player_pose",
  12: "presence_delta",
  13: "presence_ping",
  14: "presence_pong",
});

function blankDirection() {
  return { streamBytes: 0, websocketFrameBytes: 0, vxwpPayloadBytes: 0, frames: 0 };
}

function blankStats() {
  return {
    upstream: blankDirection(),
    downstream: blankDirection(),
    paths: {},
    messages: {},
  };
}

function directionFor(stats, direction, path) {
  const normalizedPath = path || "upgrade";
  stats.paths[normalizedPath] ??= {
    upstream: blankDirection(),
    downstream: blankDirection(),
  };
  return stats.paths[normalizedPath][direction];
}

function increment(target, field, amount) {
  target[field] += amount;
}

class WebSocketFrameParser {
  constructor(onMessage) {
    this.buffer = Buffer.alloc(0);
    this.fragmentOpcode = null;
    this.fragments = [];
    this.onMessage = onMessage;
  }

  push(bytes) {
    this.buffer =
      this.buffer.length === 0 ? Buffer.from(bytes) : Buffer.concat([this.buffer, bytes]);
    while (this.buffer.length >= 2) {
      const first = this.buffer[0];
      const second = this.buffer[1];
      const final = (first & 0x80) !== 0;
      const compressed = (first & 0x70) !== 0;
      const opcode = first & 0x0f;
      const masked = (second & 0x80) !== 0;
      let payloadLength = second & 0x7f;
      let offset = 2;
      if (payloadLength === 126) {
        if (this.buffer.length < 4) return;
        payloadLength = this.buffer.readUInt16BE(2);
        offset = 4;
      } else if (payloadLength === 127) {
        if (this.buffer.length < 10) return;
        const extended = this.buffer.readBigUInt64BE(2);
        if (extended > BigInt(Number.MAX_SAFE_INTEGER)) {
          throw new Error("WebSocket frame is too large to inspect safely");
        }
        payloadLength = Number(extended);
        offset = 10;
      }
      const maskBytes = masked ? 4 : 0;
      const frameLength = offset + maskBytes + payloadLength;
      if (this.buffer.length < frameLength) return;
      if (compressed)
        throw new Error("benchmark link does not accept negotiated WebSocket RSV bits");
      const frame = this.buffer.subarray(0, frameLength);
      this.buffer = this.buffer.subarray(frameLength);
      let payload = Buffer.from(frame.subarray(offset + maskBytes));
      if (masked) {
        const mask = frame.subarray(offset, offset + 4);
        for (let index = 0; index < payload.length; index += 1) {
          payload[index] ^= mask[index & 3];
        }
      }
      this.acceptFrame({ opcode, final, payload, frameLength });
    }
  }

  acceptFrame({ opcode, final, payload, frameLength }) {
    if (opcode === 0x8 || opcode === 0x9 || opcode === 0x0a) return;
    if (opcode === 0x0) {
      if (this.fragmentOpcode === null) throw new Error("unexpected continuation frame");
      this.fragments.push(payload);
      if (final) {
        const complete = Buffer.concat(this.fragments);
        const completeOpcode = this.fragmentOpcode;
        this.fragmentOpcode = null;
        this.fragments = [];
        this.onMessage(completeOpcode, complete, frameLength);
      }
      return;
    }
    if (opcode !== 0x1 && opcode !== 0x2) throw new Error(`unsupported WebSocket opcode ${opcode}`);
    if (!final) {
      if (this.fragmentOpcode !== null) throw new Error("nested fragmented WebSocket message");
      this.fragmentOpcode = opcode;
      this.fragments = [payload];
      return;
    }
    this.onMessage(opcode, payload, frameLength);
  }
}

class ConnectionInspector {
  constructor(statsRef) {
    this.statsRef = statsRef;
    this.path = "";
    this.upgraded = { upstream: false, downstream: false };
    this.http = { upstream: Buffer.alloc(0), downstream: Buffer.alloc(0) };
    this.parsers = {
      upstream: new WebSocketFrameParser((opcode, payload, frameLength) =>
        this.onMessage("upstream", opcode, payload, frameLength),
      ),
      downstream: new WebSocketFrameParser((opcode, payload, frameLength) =>
        this.onMessage("downstream", opcode, payload, frameLength),
      ),
    };
  }

  observe(direction, bytes) {
    const stats = this.statsRef.current;
    increment(stats[direction], "streamBytes", bytes.length);
    increment(directionFor(stats, direction, this.path), "streamBytes", bytes.length);
    if (this.upgraded[direction]) {
      this.parsers[direction].push(bytes);
      return;
    }
    const buffered = Buffer.concat([this.http[direction], bytes]);
    if (buffered.length > MAX_HTTP_HEADER_BYTES)
      throw new Error("WebSocket HTTP upgrade is too large");
    const end = buffered.indexOf(HTTP_HEADER_END);
    if (end < 0) {
      this.http[direction] = buffered;
      return;
    }
    const headerLength = end + HTTP_HEADER_END.length;
    if (direction === "upstream") {
      const firstLine = buffered.subarray(0, end).toString("latin1").split("\r\n", 1)[0];
      const match = /^GET\s+(\S+)\s+HTTP\/1\.[01]$/.exec(firstLine);
      if (!match) throw new Error(`could not classify WebSocket upgrade: ${firstLine}`);
      this.path = match[1];
    }
    this.upgraded[direction] = true;
    this.http[direction] = Buffer.alloc(0);
    if (buffered.length > headerLength)
      this.parsers[direction].push(buffered.subarray(headerLength));
  }

  onMessage(direction, opcode, payload, frameLength) {
    const stats = this.statsRef.current;
    const totals = stats[direction];
    const pathTotals = directionFor(stats, direction, this.path);
    increment(totals, "frames", 1);
    increment(totals, "websocketFrameBytes", frameLength);
    increment(pathTotals, "frames", 1);
    increment(pathTotals, "websocketFrameBytes", frameLength);
    if (opcode !== 0x2 || payload.length < 24 || !payload.subarray(0, 4).equals(VXWP_MAGIC)) return;
    const kind = payload.readUInt16LE(6);
    const name = VXWP_KIND_NAMES[kind] ?? `kind_${kind}`;
    const key = `${direction}:${name}`;
    stats.messages[key] ??= { direction, kind, name, frames: 0, payloadBytes: 0 };
    stats.messages[key].frames += 1;
    stats.messages[key].payloadBytes += payload.length;
    increment(totals, "vxwpPayloadBytes", payload.length);
    increment(pathTotals, "vxwpPayloadBytes", payload.length);
  }
}

export function serializationMilliseconds(bytes, megabitsPerSecond) {
  if (!(megabitsPerSecond > 0)) throw new Error("link bandwidth must be positive");
  return (bytes * 8) / (megabitsPerSecond * 1_000);
}

function defaultQueueBytes(megabitsPerSecond, oneWayLatencyMs, quantumBytes) {
  const roundTripSeconds = (oneWayLatencyMs * 2) / 1_000;
  const bandwidthDelayProduct = (megabitsPerSecond * 1_000_000 * roundTripSeconds) / 8;
  return Math.max(
    quantumBytes * 2,
    Math.ceil(bandwidthDelayProduct * QUEUE_BANDWIDTH_DELAY_PRODUCTS),
  );
}

class SerializationClock {
  constructor() {
    this.nextFinishMs = 0;
  }

  reserve(bytes, { enqueuedMs, oneWayLatencyMs, megabitsPerSecond }) {
    const readyMs = enqueuedMs + oneWayLatencyMs;
    const startMs = Math.max(readyMs, this.nextFinishMs);
    const finishMs = startMs + serializationMilliseconds(bytes, megabitsPerSecond);
    this.nextFinishMs = finishMs;
    return finishMs;
  }
}

function shapeDirection(source, destination, inspector, direction, settings) {
  const queue = [];
  let queuedBytes = 0;
  let draining = false;

  const drain = async () => {
    if (draining) return;
    draining = true;
    while (queue.length > 0 && !destination.destroyed) {
      const item = queue.shift();
      queuedBytes -= item.bytes.length;
      if (source.isPaused() && queuedBytes < settings.maxQueuedBytes / 2) source.resume();
      const delay = item.deliverAtMs - performance.now();
      if (delay > 0) await new Promise((resolve) => setTimeout(resolve, delay));
      if (destination.destroyed) break;
      inspector.observe(direction, item.bytes);
      if (!destination.write(item.bytes)) {
        await new Promise((resolve) => destination.once("drain", resolve));
      }
    }
    draining = false;
  };

  source.on("data", (chunk) => {
    for (let offset = 0; offset < chunk.length; offset += settings.quantumBytes) {
      const bytes = Buffer.from(chunk.subarray(offset, offset + settings.quantumBytes));
      const enqueuedMs = performance.now();
      queue.push({
        bytes,
        deliverAtMs: settings.clock.reserve(bytes.length, {
          enqueuedMs,
          oneWayLatencyMs: settings.oneWayLatencyMs,
          megabitsPerSecond: settings.megabitsPerSecond,
        }),
      });
      queuedBytes += bytes.length;
    }
    if (queuedBytes >= settings.maxQueuedBytes) source.pause();
    void drain();
  });
  source.on("end", () => destination.end());
  source.on("error", () => destination.destroy());
}

function clonedStats(stats) {
  return {
    upstream: { ...stats.upstream },
    downstream: { ...stats.downstream },
    paths: Object.fromEntries(
      Object.entries(stats.paths).map(([path, directions]) => [
        path,
        { upstream: { ...directions.upstream }, downstream: { ...directions.downstream } },
      ]),
    ),
    messages: Object.fromEntries(
      Object.entries(stats.messages).map(([key, value]) => [key, { ...value }]),
    ),
  };
}

export async function createShapedLink({ listenPort, targetPort, profile }) {
  const statsRef = { current: blankStats() };
  const sockets = new Set();
  const normalized = {
    oneWayLatencyMs: profile.oneWayLatencyMs,
    upstreamMegabitsPerSecond: profile.upstreamMegabitsPerSecond,
    downstreamMegabitsPerSecond: profile.downstreamMegabitsPerSecond,
    quantumBytes: profile.quantumBytes ?? DEFAULT_QUANTUM_BYTES,
  };
  normalized.upstreamMaxQueuedBytes =
    profile.upstreamMaxQueuedBytes ??
    defaultQueueBytes(
      normalized.upstreamMegabitsPerSecond,
      normalized.oneWayLatencyMs,
      normalized.quantumBytes,
    );
  normalized.downstreamMaxQueuedBytes =
    profile.downstreamMaxQueuedBytes ??
    defaultQueueBytes(
      normalized.downstreamMegabitsPerSecond,
      normalized.oneWayLatencyMs,
      normalized.quantumBytes,
    );
  const clocks = {
    upstream: new SerializationClock(),
    downstream: new SerializationClock(),
  };
  const server = createServer((client) => {
    client.setNoDelay(true);
    const backend = connect({ host: "127.0.0.1", port: targetPort });
    backend.setNoDelay(true);
    sockets.add(client);
    sockets.add(backend);
    const inspector = new ConnectionInspector(statsRef);
    shapeDirection(client, backend, inspector, "upstream", {
      oneWayLatencyMs: normalized.oneWayLatencyMs,
      megabitsPerSecond: normalized.upstreamMegabitsPerSecond,
      quantumBytes: normalized.quantumBytes,
      maxQueuedBytes: normalized.upstreamMaxQueuedBytes,
      clock: clocks.upstream,
    });
    shapeDirection(backend, client, inspector, "downstream", {
      oneWayLatencyMs: normalized.oneWayLatencyMs,
      megabitsPerSecond: normalized.downstreamMegabitsPerSecond,
      quantumBytes: normalized.quantumBytes,
      maxQueuedBytes: normalized.downstreamMaxQueuedBytes,
      clock: clocks.downstream,
    });
    const forget = (socket) => sockets.delete(socket);
    client.on("close", () => forget(client));
    backend.on("close", () => forget(backend));
    backend.on("error", () => client.destroy());
    client.on("error", () => backend.destroy());
  });
  await new Promise((resolve, reject) => {
    server.once("error", reject);
    server.listen(listenPort, "127.0.0.1", resolve);
  });
  return {
    profile: normalized,
    snapshot: () => clonedStats(statsRef.current),
    reset: () => {
      statsRef.current = blankStats();
    },
    close: async () => {
      for (const socket of sockets) socket.destroy();
      await new Promise((resolve, reject) =>
        server.close((error) => (error ? reject(error) : resolve())),
      );
    },
  };
}

export const testInternals = Object.freeze({ SerializationClock, WebSocketFrameParser });
