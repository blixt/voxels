import { readFileSync } from "node:fs";
import { strict as assert } from "node:assert";
import { describe, it } from "vite-plus/test";
import {
  normalizeWasmDeclaration,
  prependPathEntry,
  RUST_INPUT_FILES,
  RUST_SOURCE_DIRS,
  validateWasmBindgenCliVersion,
  wasmBuildIsCurrent,
} from "./build-wasm.ts";

const root = new URL("../", import.meta.url);

describe("WASM build inputs", () => {
  it("never reuses an artifact built with a different profile", () => {
    assert.equal(wasmBuildIsCurrent("wasm-dev", "wasm-dev", 20, 10, true), true);
    assert.equal(wasmBuildIsCurrent("release", "wasm-dev", 20, 10, true), false);
    assert.equal(wasmBuildIsCurrent("wasm-dev", "wasm-dev", 10, 20, true), false);
    assert.equal(wasmBuildIsCurrent("wasm-dev", "wasm-dev", 20, 10, false), false);
  });

  it("tracks every local crate in the browser shell dependency graph", () => {
    const manifest = readFileSync(new URL("Cargo.toml", root), "utf8");
    const members = manifest.match(/members\s*=\s*(\[[^\]]+\])/s)?.[1];
    assert.ok(members, "Cargo.toml must declare workspace members");

    const workspaceCrates = new Set([...members.matchAll(/"([^"]+)"/g)].map((match) => match[1]));
    assert.ok(workspaceCrates.size > 0, "Cargo.toml workspace members must not be empty");

    const reachable = new Set(["shell"]);
    const pending = ["shell"];
    while (pending.length > 0) {
      const crate = pending.pop();
      assert.ok(crate);
      const crateManifest = readFileSync(new URL(`${crate}/Cargo.toml`, root), "utf8");
      for (const match of crateManifest.matchAll(/path\s*=\s*"\.\.\/([^"]+)"/g)) {
        const dependency = match[1];
        assert.ok(dependency);
        if (!workspaceCrates.has(dependency) || reachable.has(dependency)) continue;
        reachable.add(dependency);
        pending.push(dependency);
      }
    }

    const crates = [...reachable].sort();
    assert.deepEqual([...RUST_SOURCE_DIRS].sort(), crates.map((crate) => `${crate}/src`).sort());
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
    readonly enginehandle_destroy: (a: number) => void;
    readonly memory: WebAssembly.Memory;
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

  it("prepends executable paths with the host separator and no empty entry", () => {
    assert.equal(
      prependPathEntry("C:\\cargo\\bin", "C:\\Windows", ";"),
      "C:\\cargo\\bin;C:\\Windows",
    );
    assert.equal(prependPathEntry("/cargo/bin", "", ":"), "/cargo/bin");
  });
});
