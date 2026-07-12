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
    const main = readFileSync(new URL("web/main.ts", root), "utf8");
    for (const forbidden of [
      "createElement",
      "appendChild",
      "insertAdjacentHTML",
      "innerHTML",
      "outerHTML",
      "textContent",
      "document.write",
    ]) {
      assert.equal(main.includes(forbidden), false, `${forbidden} would violate the DOM contract`);
    }
  });
});
