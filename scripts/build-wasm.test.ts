import { readFileSync } from "node:fs";
import { strict as assert } from "node:assert";
import { describe, it } from "vite-plus/test";
import {
  normalizeWasmDeclaration,
  RUST_INPUT_FILES,
  RUST_SOURCE_DIRS,
  validateWasmBindgenCliVersion,
} from "./build-wasm.ts";

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

  it("removes profile-specific internals from the public declaration", () => {
    const declaration = `export class EngineHandle {
    [Symbol.dispose](): void;
}
export interface InitOutput {
    readonly memory: WebAssembly.Memory;
    readonly wasm_bindgen_debug___convert__closures_____invoke: () => void;
    readonly enginehandle_destroy: (a: number) => void;
}`;

    assert.equal(
      normalizeWasmDeclaration(declaration),
      `export class EngineHandle {
}
export interface InitOutput {
    readonly memory: WebAssembly.Memory;
    readonly enginehandle_destroy: (a: number) => void;
}`,
    );
  });

  it("rejects missing or mismatched wasm-bindgen CLI versions with an install command", () => {
    assert.doesNotThrow(() => validateWasmBindgenCliVersion("wasm-bindgen 0.2.117\n", "0.2.117"));
    assert.throws(
      () => validateWasmBindgenCliVersion("wasm-bindgen 0.2.116\n", "0.2.117"),
      /cargo install --locked wasm-bindgen-cli --version 0\.2\.117/,
    );
    assert.throws(
      () => validateWasmBindgenCliVersion("", "0.2.117"),
      /could not read the installed version/,
    );
  });
});
