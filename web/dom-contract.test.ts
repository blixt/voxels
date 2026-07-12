import { readFileSync } from "node:fs";
import { strict as assert } from "node:assert";
import { describe, it } from "vite-plus/test";

const root = new URL("../", import.meta.url);

describe("canvas-only browser shell", () => {
  it("keeps the canvas as the only rendered body element", () => {
    const html = readFileSync(new URL("index.html", root), "utf8");
    const body = html.match(/<body>([\s\S]*?)<\/body>/i)?.[1];
    assert.ok(body, "index.html must have a body");
    const tags = Array.from(body.matchAll(/<([a-z][a-z0-9-]*)(?:\s|>)/gi), (match) =>
      match[1]?.toLowerCase(),
    );
    assert.deepEqual(tags, ["canvas", "script"]);
    assert.match(body, /<canvas\s+id="app"[^>]*><\/canvas>/i);
    assert.match(body, /<script\s+type="module"\s+src="\/web\/main\.ts"><\/script>/i);
  });

  it("does not construct a JavaScript UI tree", () => {
    for (const source of ["web/main.ts", "web/worker.ts", "web/protocol.ts"]) {
      const contents = readFileSync(new URL(source, root), "utf8");
      for (const forbidden of [
        "createElement",
        "appendChild",
        "insertAdjacentHTML",
        "innerHTML",
        "outerHTML",
        "textContent",
        "document.write",
      ]) {
        assert.equal(
          contents.includes(forbidden),
          false,
          `${source} uses ${forbidden}, which would violate the DOM contract`,
        );
      }
    }
  });

  it("prevents development and production tooling from injecting DOM", () => {
    const config = readFileSync(new URL("vite.config.ts", root), "utf8");
    assert.match(config, /hmr:\s*\{\s*overlay:\s*false\s*\}/);
    assert.match(config, /modulePreload:\s*\{\s*polyfill:\s*false\s*\}/);
  });
});
