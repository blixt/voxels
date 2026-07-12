import { describe, expect, it } from "vite-plus/test";
import { watchDevicePixelRatio } from "./display.ts";

describe("display density tracking", () => {
  it("re-arms the resolution query and reports every device pixel ratio change", () => {
    let ratio = 1;
    let changes = 0;
    const queries: Array<{ query: string; listeners: Set<() => void> }> = [];
    const stop = watchDevicePixelRatio(
      () => ratio,
      (query) => {
        const listeners = new Set<() => void>();
        queries.push({ query, listeners });
        return {
          addEventListener: (_type, listener) => listeners.add(listener),
          removeEventListener: (_type, listener) => listeners.delete(listener),
        };
      },
      () => {
        changes += 1;
      },
    );

    expect(queries.map(({ query }) => query)).toEqual(["(resolution: 1dppx)"]);
    ratio = 2;
    for (const listener of queries[0]?.listeners ?? []) listener();
    expect(changes).toBe(1);
    expect(queries.map(({ query }) => query)).toEqual([
      "(resolution: 1dppx)",
      "(resolution: 2dppx)",
    ]);
    expect(queries[0]?.listeners.size).toBe(0);

    stop();
    expect(queries[1]?.listeners.size).toBe(0);
  });
});
