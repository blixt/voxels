const PNG_SIGNATURE = new Uint8Array([137, 80, 78, 71, 13, 10, 26, 10]);
const TEXT_CHUNK = new Uint8Array([116, 69, 88, 116]);
const IHDR_CHUNK = "IHDR";

function bytesEqual(left: Uint8Array, right: Uint8Array): boolean {
  return left.length === right.length && left.every((byte, index) => byte === right[index]);
}

function readU32(bytes: Uint8Array, offset: number): number {
  return (
    bytes[offset]! * 0x1000000 +
    bytes[offset + 1]! * 0x10000 +
    bytes[offset + 2]! * 0x100 +
    bytes[offset + 3]!
  );
}

function writeU32(bytes: Uint8Array, offset: number, value: number): void {
  bytes[offset] = (value >>> 24) & 0xff;
  bytes[offset + 1] = (value >>> 16) & 0xff;
  bytes[offset + 2] = (value >>> 8) & 0xff;
  bytes[offset + 3] = value & 0xff;
}

function chunkType(bytes: Uint8Array, offset: number): string {
  return String.fromCharCode(
    bytes[offset]!,
    bytes[offset + 1]!,
    bytes[offset + 2]!,
    bytes[offset + 3]!,
  );
}

function crc32(bytes: Uint8Array): number {
  let crc = 0xffffffff;
  for (const byte of bytes) {
    crc ^= byte;
    for (let bit = 0; bit < 8; bit += 1) {
      crc = (crc >>> 1) ^ (0xedb88320 & -(crc & 1));
    }
  }
  return (crc ^ 0xffffffff) >>> 0;
}

function assertPng(bytes: Uint8Array): void {
  if (bytes.length < PNG_SIGNATURE.length || !bytesEqual(bytes.subarray(0, 8), PNG_SIGNATURE)) {
    throw new Error("screenshot encoder returned an invalid PNG signature");
  }
}

function everyCodeUnit(value: string, predicate: (code: number) => boolean): boolean {
  for (let index = 0; index < value.length; index += 1) {
    if (!predicate(value.charCodeAt(index))) return false;
  }
  return true;
}

function chunkEnd(bytes: Uint8Array, offset: number): number {
  if (offset + 12 > bytes.length) throw new Error("PNG chunk header is truncated");
  const end = offset + 12 + readU32(bytes, offset);
  if (end > bytes.length) throw new Error("PNG chunk payload is truncated");
  return end;
}

/**
 * Inserts a standards-compatible PNG tEXt chunk immediately after IHDR.
 *
 * Reproduction metadata is deliberately ASCII JSON so command-line tools and image libraries can
 * recover it without a Voxels-specific sidecar file.
 */
export function embedPngText(png: Uint8Array, keyword: string, text: string): Uint8Array {
  assertPng(png);
  if (
    keyword.length === 0 ||
    keyword.length > 79 ||
    !everyCodeUnit(keyword, (code) => code >= 32 && code <= 126)
  ) {
    throw new Error("PNG text keyword must contain 1-79 printable ASCII characters");
  }
  if (!everyCodeUnit(text, (code) => code <= 127)) {
    throw new Error("PNG reproduction metadata must be ASCII");
  }
  const ihdrOffset = PNG_SIGNATURE.length;
  if (chunkType(png, ihdrOffset + 4) !== IHDR_CHUNK) {
    throw new Error("PNG does not begin with IHDR");
  }
  const insertionOffset = chunkEnd(png, ihdrOffset);
  const payload = new TextEncoder().encode(`${keyword}\0${text}`);
  const chunk = new Uint8Array(payload.length + 12);
  writeU32(chunk, 0, payload.length);
  chunk.set(TEXT_CHUNK, 4);
  chunk.set(payload, 8);
  writeU32(chunk, chunk.length - 4, crc32(chunk.subarray(4, chunk.length - 4)));

  const encoded = new Uint8Array(png.length + chunk.length);
  encoded.set(png.subarray(0, insertionOffset));
  encoded.set(chunk, insertionOffset);
  encoded.set(png.subarray(insertionOffset), insertionOffset + chunk.length);
  return encoded;
}

/** Returns the first matching PNG tEXt value, or undefined when the keyword is absent. */
export function readPngText(png: Uint8Array, keyword: string): string | undefined {
  assertPng(png);
  const decoder = new TextDecoder("latin1");
  let offset = PNG_SIGNATURE.length;
  while (offset < png.length) {
    const end = chunkEnd(png, offset);
    const type = chunkType(png, offset + 4);
    if (type === "tEXt") {
      const payload = png.subarray(offset + 8, end - 4);
      const separator = payload.indexOf(0);
      if (separator > 0 && decoder.decode(payload.subarray(0, separator)) === keyword) {
        return decoder.decode(payload.subarray(separator + 1));
      }
    }
    if (type === "IEND") break;
    offset = end;
  }
  return undefined;
}
