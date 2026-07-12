import { describe, expect, it } from "vite-plus/test";
import { watchRustInputChanges } from "./vite.config.ts";

describe("Rust WASM development watcher", () => {
  it("rebuilds for changed, added, and removed Rust inputs", () => {
    const registrations = new Map<string, (file: string) => void>();
    const listener = (): void => undefined;

    watchRustInputChanges(
      {
        on: (event, registered) => registrations.set(event, registered),
      },
      listener,
    );

    expect([...registrations.keys()]).toEqual(["add", "change", "unlink"]);
    expect([...registrations.values()]).toEqual([listener, listener, listener]);
  });
});
