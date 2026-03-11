export function average(values: number[]): number {
  if (values.length === 0) {
    return 0;
  }
  let total = 0;
  for (const value of values) {
    total += value;
  }
  return total / values.length;
}

export function firstValue<T>(values: readonly T[]): T | null {
  return values[0] ?? null;
}

export function averageWarm(values: readonly number[]): number | null {
  if (values.length < 2) {
    return null;
  }
  let total = 0;
  for (let index = 1; index < values.length; index += 1) {
    total += values[index]!;
  }
  return total / (values.length - 1);
}

export function maxValue(values: readonly number[]): number {
  let maximum = 0;
  for (const value of values) {
    if (value > maximum) {
      maximum = value;
    }
  }
  return maximum;
}

export function percentile(values: readonly number[], fraction: number): number {
  if (values.length === 0) {
    return 0;
  }
  const clamped = Math.min(Math.max(fraction, 0), 1);
  const sorted = [...values].sort((left, right) => left - right);
  const index = clamped === 0
    ? 0
    : Math.min(sorted.length - 1, Math.ceil(clamped * sorted.length) - 1);
  return sorted[index] ?? 0;
}
