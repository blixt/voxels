const KEY_CODES: Readonly<Record<string, number>> = {
  KeyW: 1,
  KeyA: 2,
  KeyS: 3,
  KeyD: 4,
  Space: 5,
  ShiftLeft: 6,
  ShiftRight: 6,
  F3: 8,
};

export function keyCode(code: string): number {
  return KEY_CODES[code] ?? 0;
}

export async function requestPointerLockSafely(
  request: () => Promise<void>,
  onFailure: (error: unknown) => void,
): Promise<void> {
  try {
    await request();
  } catch (error) {
    onFailure(error);
  }
}

const WHEEL_SELECTION_THRESHOLD_PIXELS = 100;

/** Normalizes mouse wheels and high-resolution trackpads into deliberate inventory selections. */
export class WheelAccumulator {
  #pixels = 0;

  consume(deltaY: number, deltaMode: number, pageHeight: number): number[] {
    if (!Number.isFinite(deltaY) || deltaY === 0) return [];
    const multiplier = deltaMode === 1 ? 34 : deltaMode === 2 ? Math.max(pageHeight, 1) : 1;
    const pixels = Math.max(
      -WHEEL_SELECTION_THRESHOLD_PIXELS * 4,
      Math.min(WHEEL_SELECTION_THRESHOLD_PIXELS * 4, deltaY * multiplier),
    );
    if (this.#pixels !== 0 && Math.sign(this.#pixels) !== Math.sign(pixels)) this.#pixels = 0;
    this.#pixels += pixels;
    const directions: number[] = [];
    while (Math.abs(this.#pixels) >= WHEEL_SELECTION_THRESHOLD_PIXELS) {
      const direction = Math.sign(this.#pixels);
      directions.push(direction);
      this.#pixels -= direction * WHEEL_SELECTION_THRESHOLD_PIXELS;
    }
    return directions;
  }

  clear(): void {
    this.#pixels = 0;
  }
}

/** Keeps aliased physical keys pressed until every key for the logical input is released. */
export class PressedKeys {
  readonly #physical = new Set<string>();

  keyDown(code: string): number {
    const logical = keyCode(code);
    if (logical !== 0) this.#physical.add(code);
    return logical;
  }

  keyUp(code: string): number {
    const logical = keyCode(code);
    if (logical === 0) return 0;
    this.#physical.delete(code);
    for (const pressed of this.#physical) {
      if (keyCode(pressed) === logical) return 0;
    }
    return logical;
  }

  clear(): void {
    this.#physical.clear();
  }
}
