export interface HotbarWindowLayout {
  startSlot: number;
  endSlotExclusive: number;
  leftHiddenCount: number;
  rightHiddenCount: number;
}

export function describeHotbarWindow(
  selectedSlot: number,
  slotCount: number,
  visibleCount: number,
): HotbarWindowLayout {
  if (slotCount <= 0 || visibleCount <= 0) {
    return {
      startSlot: 0,
      endSlotExclusive: 0,
      leftHiddenCount: 0,
      rightHiddenCount: 0,
    };
  }
  const clampedVisible = Math.min(slotCount, visibleCount);
  const centered = selectedSlot - Math.floor(clampedVisible / 2);
  const startSlot = Math.max(0, Math.min(slotCount - clampedVisible, centered));
  const endSlotExclusive = startSlot + clampedVisible;
  return {
    startSlot,
    endSlotExclusive,
    leftHiddenCount: startSlot,
    rightHiddenCount: Math.max(0, slotCount - endSlotExclusive),
  };
}
