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
