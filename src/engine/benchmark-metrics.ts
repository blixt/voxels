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
