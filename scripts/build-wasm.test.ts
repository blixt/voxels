import { readFileSync } from "node:fs";
import { strict as assert } from "node:assert";
import { describe, it } from "vite-plus/test";
import { RUST_INPUT_FILES, RUST_SOURCE_DIRS } from "./build-wasm.ts";

const root = new URL("../", import.meta.url);

describe("WASM build inputs", () => {
  it("tracks every Rust workspace crate", () => {
    const manifest = readFileSync(new URL("Cargo.toml", root), "utf8");
    const members = manifest.match(/members\s*=\s*(\[[^\]]+\])/s)?.[1];
    assert.ok(members, "Cargo.toml must declare workspace members");

    const crates = JSON.parse(members) as string[];
    assert.deepEqual(
      RUST_SOURCE_DIRS,
      crates.map((crate) => `${crate}/src`),
    );
    for (const crate of crates) {
      assert.ok(
        RUST_INPUT_FILES.includes(`${crate}/Cargo.toml`),
        `${crate}/Cargo.toml must invalidate the WASM build`,
      );
    }
  });
});
