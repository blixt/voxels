export const INVENTORY_STACK_LIMIT = 32;
export const INVENTORY_STACK_SIZE = 1024;

export interface InventoryStack {
  material: number;
  count: number;
}

export interface InventoryState {
  selectedSlot: number;
  slots: Array<InventoryStack | null>;
}

export interface InventoryInsertResult {
  added: number;
  leftover: number;
}

export interface InventoryRemoveResult {
  removed: number;
  emptySlot: boolean;
}

export function createInventoryState(): InventoryState {
  return {
    selectedSlot: 0,
    slots: new Array<InventoryStack | null>(INVENTORY_STACK_LIMIT).fill(null),
  };
}

export function selectInventorySlot(state: InventoryState, slot: number): number {
  const normalized = Math.max(0, Math.min(INVENTORY_STACK_LIMIT - 1, Math.floor(slot)));
  state.selectedSlot = normalized;
  return normalized;
}

export function cycleInventorySlot(state: InventoryState, delta: number): number {
  const normalizedDelta = delta < 0 ? -1 : delta > 0 ? 1 : 0;
  if (normalizedDelta === 0) {
    return state.selectedSlot;
  }
  const next = (state.selectedSlot + normalizedDelta + INVENTORY_STACK_LIMIT) % INVENTORY_STACK_LIMIT;
  state.selectedSlot = next;
  return next;
}

export function insertInventoryMaterial(state: InventoryState, material: number, count = 1): InventoryInsertResult {
  if (material === 0 || count <= 0) {
    return { added: 0, leftover: count };
  }
  let remaining = count;
  for (const stack of state.slots) {
    if (!stack || stack.material !== material || stack.count >= INVENTORY_STACK_SIZE) {
      continue;
    }
    const accepted = Math.min(INVENTORY_STACK_SIZE - stack.count, remaining);
    stack.count += accepted;
    remaining -= accepted;
    if (remaining === 0) {
      return { added: count, leftover: 0 };
    }
  }
  for (let index = 0; index < state.slots.length; index += 1) {
    if (state.slots[index]) {
      continue;
    }
    const accepted = Math.min(INVENTORY_STACK_SIZE, remaining);
    state.slots[index] = { material, count: accepted };
    remaining -= accepted;
    if (remaining === 0) {
      return { added: count, leftover: 0 };
    }
  }
  return {
    added: count - remaining,
    leftover: remaining,
  };
}

export function getInventoryInsertCapacity(state: InventoryState, material: number): number {
  if (material === 0) {
    return 0;
  }
  let capacity = 0;
  for (const stack of state.slots) {
    if (!stack) {
      capacity += INVENTORY_STACK_SIZE;
      continue;
    }
    if (stack.material === material) {
      capacity += INVENTORY_STACK_SIZE - stack.count;
    }
  }
  return capacity;
}

export function removeSelectedInventoryMaterial(state: InventoryState, count = 1): InventoryRemoveResult {
  const stack = state.slots[state.selectedSlot];
  if (!stack || count <= 0) {
    return {
      removed: 0,
      emptySlot: !stack,
    };
  }
  const removed = Math.min(stack.count, count);
  stack.count -= removed;
  const emptySlot = stack.count === 0;
  if (emptySlot) {
    state.slots[state.selectedSlot] = null;
  }
  return {
    removed,
    emptySlot,
  };
}

export function getSelectedInventoryStack(state: InventoryState): InventoryStack | null {
  return state.slots[state.selectedSlot];
}

export function countUsedInventoryStacks(state: InventoryState): number {
  let used = 0;
  for (const stack of state.slots) {
    if (stack) {
      used += 1;
    }
  }
  return used;
}
