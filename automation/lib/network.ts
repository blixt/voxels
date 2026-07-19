import { createServer, connect } from "node:net";
import type { Socket } from "node:net";
import type { Duplex } from "node:stream";

const HTTP_HEADER_END = Buffer.from("\r\n\r\n");
const MAX_HTTP_HEADER_BYTES = 64 * 1024;
const VXWP_MAGIC = Buffer.from("VXWP");
const VXWP_HEADER_BYTES = 24;
const DEFAULT_QUANTUM_BYTES = 16 * 1024;
const QUEUE_BANDWIDTH_DELAY_PRODUCTS = 2;

export const VXWP_KIND = Object.freeze({
  openWorld: 1,
  worldOpened: 2,
  chunkBatch: 3,
  chunkBatchResult: 4,
  cancel: 5,
  error: 6,
  surfaceTileBatch: 7,
  surfaceTileBatchResult: 8,
  openPresence: 9,
  presenceOpened: 10,
  playerPose: 11,
  presenceDelta: 12,
  presencePing: 13,
  presencePong: 14,
  editCommand: 15,
  editCommit: 16,
  resyncRequired: 17,
  frameFragment: 18,
} as const);

export type VxwpKind = (typeof VXWP_KIND)[keyof typeof VXWP_KIND];

export const VXWP_KIND_NAMES = Object.freeze({
  [VXWP_KIND.openWorld]: "open_world",
  [VXWP_KIND.worldOpened]: "world_opened",
  [VXWP_KIND.chunkBatch]: "chunk_batch",
  [VXWP_KIND.chunkBatchResult]: "chunk_batch_result",
  [VXWP_KIND.cancel]: "cancel",
  [VXWP_KIND.error]: "error",
  [VXWP_KIND.surfaceTileBatch]: "surface_tile_batch",
  [VXWP_KIND.surfaceTileBatchResult]: "surface_tile_batch_result",
  [VXWP_KIND.openPresence]: "open_presence",
  [VXWP_KIND.presenceOpened]: "presence_opened",
  [VXWP_KIND.playerPose]: "player_pose",
  [VXWP_KIND.presenceDelta]: "presence_delta",
  [VXWP_KIND.presencePing]: "presence_ping",
  [VXWP_KIND.presencePong]: "presence_pong",
  [VXWP_KIND.editCommand]: "edit_command",
  [VXWP_KIND.editCommit]: "edit_commit",
  [VXWP_KIND.resyncRequired]: "resync_required",
  [VXWP_KIND.frameFragment]: "frame_fragment",
} satisfies Record<VxwpKind, string>);

export type LinkDirection = "upstream" | "downstream";

export interface LinkDirectionStats {
  streamBytes: number;
  websocketFrameBytes: number;
  vxwpPayloadBytes: number;
  frames: number;
  peakQueuedBytes: number;
  peakQueueDelayMs: number;
  backpressurePauses: number;
}

export interface LinkMessageStats {
  direction: LinkDirection;
  kind: number;
  name: string;
  frames: number;
  payloadBytes: number;
  maxPayloadBytes: number;
  lastObservedRoundTripMs?: number;
  maxObservedRoundTripMs?: number;
  lastOutboundRateBytesPerSecond?: number;
  maxOutboundRateBytesPerSecond?: number;
  worldProductPriorityFrames?: Record<string, number>;
}

export interface LinkStats {
  upstream: LinkDirectionStats;
  downstream: LinkDirectionStats;
  paths: Record<string, { upstream: LinkDirectionStats; downstream: LinkDirectionStats }>;
  messages: Record<string, LinkMessageStats>;
}

export interface ShapedLinkProfile {
  readonly name?: string;
  readonly oneWayLatencyMs: number;
  readonly upstreamMegabitsPerSecond: number;
  readonly downstreamMegabitsPerSecond: number;
  readonly quantumBytes?: number;
  readonly upstreamMaxQueuedBytes?: number;
  readonly downstreamMaxQueuedBytes?: number;
}

export interface NormalizedShapedLinkProfile {
  readonly oneWayLatencyMs: number;
  readonly upstreamMegabitsPerSecond: number;
  readonly downstreamMegabitsPerSecond: number;
  readonly quantumBytes: number;
  readonly upstreamMaxQueuedBytes: number;
  readonly downstreamMaxQueuedBytes: number;
}

export interface ShapedLink {
  readonly port: number;
  readonly profile: NormalizedShapedLinkProfile;
  snapshot(): LinkStats;
  reset(): void;
  close(): Promise<void>;
}

type DirectionStatField = keyof LinkDirectionStats;
type FrameCallback = (
  opcode: number,
  payload: Buffer,
  frameBytes: number,
  frameCount: number,
) => void;

function blankDirection(): LinkDirectionStats {
  return {
    streamBytes: 0,
    websocketFrameBytes: 0,
    vxwpPayloadBytes: 0,
    frames: 0,
    peakQueuedBytes: 0,
    peakQueueDelayMs: 0,
    backpressurePauses: 0,
  };
}

function blankStats(): LinkStats {
  return {
    upstream: blankDirection(),
    downstream: blankDirection(),
    paths: {},
    messages: {},
  };
}

function directionFor(
  stats: LinkStats,
  direction: LinkDirection,
  path: string,
): LinkDirectionStats {
  const normalizedPath = path || "upgrade";
  stats.paths[normalizedPath] ??= {
    upstream: blankDirection(),
    downstream: blankDirection(),
  };
  return stats.paths[normalizedPath][direction];
}

function increment(target: LinkDirectionStats, field: DirectionStatField, amount: number): void {
  target[field] += amount;
}

function worldProductPriorityName(payload: Buffer): string | null {
  if (payload.length <= VXWP_HEADER_BYTES) return null;
  switch (payload[VXWP_HEADER_BYTES]) {
    case 1:
      return "collision_critical";
    case 2:
      return "visible_chunk";
    case 3:
      return "visible_surface";
    case 4:
      return "replacement_surface";
    case 5:
      return "prefetch";
    default:
      return null;
  }
}

class WebSocketFrameParser {
  buffer = Buffer.alloc(0);
  fragmentOpcode: number | null = null;
  fragments: Buffer[] = [];
  fragmentFrameBytes = 0;
  fragmentFrameCount = 0;
  readonly onMessage: FrameCallback;

  constructor(onMessage: FrameCallback) {
    this.onMessage = onMessage;
  }

  push(bytes: Uint8Array): void {
    this.buffer =
      this.buffer.length === 0 ? Buffer.from(bytes) : Buffer.concat([this.buffer, bytes]);
    while (this.buffer.length >= 2) {
      const first = this.buffer[0];
      const second = this.buffer[1];
      if (first === undefined || second === undefined) return;
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
          const payloadByte = payload[index];
          const maskByte = mask[index & 3];
          if (payloadByte === undefined || maskByte === undefined) {
            throw new Error("masked WebSocket frame is incomplete");
          }
          payload[index] = payloadByte ^ maskByte;
        }
      }
      this.acceptFrame({ opcode, final, payload, frameLength });
    }
  }

  acceptFrame({
    opcode,
    final,
    payload,
    frameLength,
  }: {
    readonly opcode: number;
    readonly final: boolean;
    readonly payload: Buffer;
    readonly frameLength: number;
  }): void {
    if (opcode === 0x8 || opcode === 0x9 || opcode === 0x0a) return;
    if (opcode === 0x0) {
      if (this.fragmentOpcode === null) throw new Error("unexpected continuation frame");
      this.fragments.push(payload);
      this.fragmentFrameBytes += frameLength;
      this.fragmentFrameCount += 1;
      if (final) {
        const complete = Buffer.concat(this.fragments);
        const completeOpcode = this.fragmentOpcode;
        const completeFrameBytes = this.fragmentFrameBytes;
        const completeFrameCount = this.fragmentFrameCount;
        this.fragmentOpcode = null;
        this.fragments = [];
        this.fragmentFrameBytes = 0;
        this.fragmentFrameCount = 0;
        this.onMessage(completeOpcode, complete, completeFrameBytes, completeFrameCount);
      }
      return;
    }
    if (opcode !== 0x1 && opcode !== 0x2) throw new Error(`unsupported WebSocket opcode ${opcode}`);
    if (!final) {
      if (this.fragmentOpcode !== null) throw new Error("nested fragmented WebSocket message");
      this.fragmentOpcode = opcode;
      this.fragments = [payload];
      this.fragmentFrameBytes = frameLength;
      this.fragmentFrameCount = 1;
      return;
    }
    this.onMessage(opcode, payload, frameLength, 1);
  }
}

class ConnectionInspector {
  readonly statsRef: { current: LinkStats };
  path = "";
  readonly upgraded: Record<LinkDirection, boolean> = { upstream: false, downstream: false };
  readonly http: Record<LinkDirection, Buffer> = {
    upstream: Buffer.alloc(0),
    downstream: Buffer.alloc(0),
  };
  readonly parsers: Record<LinkDirection, WebSocketFrameParser>;

  constructor(statsRef: { current: LinkStats }) {
    this.statsRef = statsRef;
    this.parsers = {
      upstream: new WebSocketFrameParser((opcode, payload, frameBytes, frameCount) =>
        this.onMessage("upstream", opcode, payload, frameBytes, frameCount),
      ),
      downstream: new WebSocketFrameParser((opcode, payload, frameBytes, frameCount) =>
        this.onMessage("downstream", opcode, payload, frameBytes, frameCount),
      ),
    };
  }

  observe(direction: LinkDirection, bytes: Buffer): void {
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
      if (firstLine === undefined) throw new Error("WebSocket upgrade omitted its request line");
      const match = /^GET\s+(\S+)\s+HTTP\/1\.[01]$/.exec(firstLine);
      if (!match) throw new Error(`could not classify WebSocket upgrade: ${firstLine}`);
      this.path = match[1] ?? "";
    }
    this.upgraded[direction] = true;
    this.http[direction] = Buffer.alloc(0);
    if (buffered.length > headerLength)
      this.parsers[direction].push(buffered.subarray(headerLength));
  }

  onMessage(
    direction: LinkDirection,
    opcode: number,
    payload: Buffer,
    frameBytes: number,
    frameCount: number,
  ): void {
    const stats = this.statsRef.current;
    const totals = stats[direction];
    const pathTotals = directionFor(stats, direction, this.path);
    increment(totals, "frames", frameCount);
    increment(totals, "websocketFrameBytes", frameBytes);
    increment(pathTotals, "frames", frameCount);
    increment(pathTotals, "websocketFrameBytes", frameBytes);
    if (
      opcode !== 0x2 ||
      payload.length < VXWP_HEADER_BYTES ||
      !payload.subarray(0, 4).equals(VXWP_MAGIC)
    )
      return;
    const kind = payload.readUInt16LE(6);
    const name = (VXWP_KIND_NAMES as Readonly<Record<number, string>>)[kind] ?? `kind_${kind}`;
    const key = `${direction}:${name}`;
    stats.messages[key] ??= {
      direction,
      kind,
      name,
      frames: 0,
      payloadBytes: 0,
      maxPayloadBytes: 0,
    };
    stats.messages[key].frames += 1;
    stats.messages[key].payloadBytes += payload.length;
    stats.messages[key].maxPayloadBytes = Math.max(
      stats.messages[key].maxPayloadBytes,
      payload.length,
    );
    if (
      direction === "upstream" &&
      (kind === VXWP_KIND.chunkBatch || kind === VXWP_KIND.surfaceTileBatch)
    ) {
      const priority = worldProductPriorityName(payload);
      if (priority !== null) {
        stats.messages[key].worldProductPriorityFrames ??= {};
        stats.messages[key].worldProductPriorityFrames[priority] =
          (stats.messages[key].worldProductPriorityFrames[priority] ?? 0) + 1;
      }
    }
    if (direction === "upstream" && kind === VXWP_KIND.presencePing && payload.length === 40) {
      const roundTripMs = payload.readUInt32LE(28);
      stats.messages[key].lastObservedRoundTripMs = roundTripMs;
      stats.messages[key].maxObservedRoundTripMs = Math.max(
        stats.messages[key].maxObservedRoundTripMs ?? 0,
        roundTripMs,
      );
    }
    if (direction === "downstream" && kind === VXWP_KIND.presencePong && payload.length === 56) {
      const rate = payload.readUInt32LE(28);
      stats.messages[key].lastOutboundRateBytesPerSecond = rate;
      stats.messages[key].maxOutboundRateBytesPerSecond = Math.max(
        stats.messages[key].maxOutboundRateBytesPerSecond ?? 0,
        rate,
      );
    }
    increment(totals, "vxwpPayloadBytes", payload.length);
    increment(pathTotals, "vxwpPayloadBytes", payload.length);
  }

  observeQueue(direction: LinkDirection, queuedBytes: number, queueDelayMs: number): void {
    const stats = this.statsRef.current;
    for (const totals of [stats[direction], directionFor(stats, direction, this.path)]) {
      totals.peakQueuedBytes = Math.max(totals.peakQueuedBytes, queuedBytes);
      totals.peakQueueDelayMs = Math.max(totals.peakQueueDelayMs, queueDelayMs);
    }
  }

  observeBackpressure(direction: LinkDirection): void {
    const stats = this.statsRef.current;
    increment(stats[direction], "backpressurePauses", 1);
    increment(directionFor(stats, direction, this.path), "backpressurePauses", 1);
  }
}

export function serializationMilliseconds(bytes: number, megabitsPerSecond: number): number {
  if (!(megabitsPerSecond > 0)) throw new Error("link bandwidth must be positive");
  return (bytes * 8) / (megabitsPerSecond * 1_000);
}

function defaultQueueBytes(
  megabitsPerSecond: number,
  oneWayLatencyMs: number,
  quantumBytes: number,
): number {
  const roundTripSeconds = (oneWayLatencyMs * 2) / 1_000;
  const bandwidthDelayProduct = (megabitsPerSecond * 1_000_000 * roundTripSeconds) / 8;
  return Math.max(
    quantumBytes * 2,
    Math.ceil(bandwidthDelayProduct * QUEUE_BANDWIDTH_DELAY_PRODUCTS),
  );
}

class SerializationClock {
  nextFinishMs = 0;

  reserve(
    bytes: number,
    {
      enqueuedMs,
      oneWayLatencyMs,
      megabitsPerSecond,
    }: {
      readonly enqueuedMs: number;
      readonly oneWayLatencyMs: number;
      readonly megabitsPerSecond: number;
    },
  ): number {
    const readyMs = enqueuedMs + oneWayLatencyMs;
    const startMs = Math.max(readyMs, this.nextFinishMs);
    const finishMs = startMs + serializationMilliseconds(bytes, megabitsPerSecond);
    this.nextFinishMs = finishMs;
    return finishMs;
  }
}

interface QueuedChunk {
  readonly bytes: Buffer;
  readonly deliverAtMs: number;
}

interface DirectionSettings {
  readonly oneWayLatencyMs: number;
  readonly megabitsPerSecond: number;
  readonly quantumBytes: number;
  readonly maxQueuedBytes: number;
  readonly clock: SerializationClock;
}

interface TrafficInspector {
  observe(direction: LinkDirection, bytes: Uint8Array): void;
  observeQueue(direction: LinkDirection, queuedBytes: number, queueDelayMs: number): void;
  observeBackpressure(direction: LinkDirection): void;
}

function shapeDirection(
  source: Duplex,
  destination: Duplex,
  inspector: TrafficInspector,
  direction: LinkDirection,
  settings: DirectionSettings,
): void {
  const queue: QueuedChunk[] = [];
  let queuedBytes = 0;
  let draining = false;
  let sourceEnded = false;

  const drain = async (): Promise<void> => {
    if (draining) return;
    draining = true;
    while (queue.length > 0 && !destination.destroyed) {
      const item = queue.shift();
      if (item === undefined) break;
      queuedBytes -= item.bytes.length;
      if (source.isPaused() && queuedBytes < settings.maxQueuedBytes / 2) source.resume();
      const delay = item.deliverAtMs - performance.now();
      if (delay > 0) await new Promise<void>((resolve) => setTimeout(resolve, delay));
      if (destination.destroyed) break;
      inspector.observe(direction, item.bytes);
      if (!destination.write(item.bytes)) {
        await new Promise<void>((resolve) => destination.once("drain", resolve));
      }
    }
    draining = false;
    if (sourceEnded && !destination.destroyed) destination.end();
  };

  source.on("data", (chunk: Buffer) => {
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
      const queueDelayMs = Math.max(
        0,
        (queue.at(-1)?.deliverAtMs ?? enqueuedMs) -
          enqueuedMs -
          settings.oneWayLatencyMs -
          serializationMilliseconds(bytes.length, settings.megabitsPerSecond),
      );
      inspector.observeQueue(direction, queuedBytes, queueDelayMs);
    }
    if (queuedBytes >= settings.maxQueuedBytes) {
      source.pause();
      inspector.observeBackpressure(direction);
    }
    void drain();
  });
  // TCP EOF is ordered after every byte already read from the source. Preserve that ordering
  // across artificial latency instead of truncating the delayed queue as soon as `end` fires.
  source.on("end", () => {
    sourceEnded = true;
    void drain();
  });
  source.on("error", () => destination.destroy());
}

function clonedStats(stats: LinkStats): LinkStats {
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
      Object.entries(stats.messages).map(([key, value]) => [
        key,
        {
          ...value,
          worldProductPriorityFrames:
            value.worldProductPriorityFrames === undefined
              ? undefined
              : { ...value.worldProductPriorityFrames },
        },
      ]),
    ),
  };
}

export async function createShapedLink({
  listenPort,
  targetPort,
  profile,
}: {
  readonly listenPort: number;
  readonly targetPort: number;
  readonly profile: ShapedLinkProfile;
}): Promise<ShapedLink> {
  const statsRef = { current: blankStats() };
  const sockets = new Set<Socket>();
  const normalized: NormalizedShapedLinkProfile = {
    oneWayLatencyMs: profile.oneWayLatencyMs,
    upstreamMegabitsPerSecond: profile.upstreamMegabitsPerSecond,
    downstreamMegabitsPerSecond: profile.downstreamMegabitsPerSecond,
    quantumBytes: profile.quantumBytes ?? DEFAULT_QUANTUM_BYTES,
    upstreamMaxQueuedBytes:
      profile.upstreamMaxQueuedBytes ??
      defaultQueueBytes(
        profile.upstreamMegabitsPerSecond,
        profile.oneWayLatencyMs,
        profile.quantumBytes ?? DEFAULT_QUANTUM_BYTES,
      ),
    downstreamMaxQueuedBytes:
      profile.downstreamMaxQueuedBytes ??
      defaultQueueBytes(
        profile.downstreamMegabitsPerSecond,
        profile.oneWayLatencyMs,
        profile.quantumBytes ?? DEFAULT_QUANTUM_BYTES,
      ),
  };
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
    const forget = (socket: Socket): void => {
      sockets.delete(socket);
    };
    client.on("close", () => forget(client));
    backend.on("close", () => forget(backend));
    backend.on("error", () => client.destroy());
    client.on("error", () => backend.destroy());
  });
  await new Promise<void>((resolve, reject) => {
    server.once("error", reject);
    server.listen(listenPort, "127.0.0.1", resolve);
  });
  let closePromise: Promise<void> | undefined;
  return {
    port: listenPort,
    profile: normalized,
    snapshot: () => clonedStats(statsRef.current),
    reset: () => {
      statsRef.current = blankStats();
    },
    close: () => {
      closePromise ??= (async () => {
        for (const socket of sockets) socket.destroy();
        await new Promise<void>((resolve, reject) =>
          server.close((error) => (error ? reject(error) : resolve())),
        );
      })();
      return closePromise;
    },
  };
}

export const testInternals = Object.freeze({
  SerializationClock,
  WebSocketFrameParser,
  shapeDirection,
  worldProductPriorityName,
});
