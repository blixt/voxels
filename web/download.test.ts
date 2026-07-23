import { strict as assert } from "node:assert";
import { describe, it } from "vite-plus/test";
import { downloadBlob } from "./download.ts";

describe("browser downloads", () => {
  it("downloads a nonempty blob with a sanitized filename and releases its URL", async () => {
    let clicked = false;
    const anchor = {
      href: "",
      download: "",
      rel: "",
      click: () => {
        clicked = true;
      },
    };
    let revoked = "";
    const documentRef = {
      createElement: (tag: string) => {
        assert.equal(tag, "a");
        return anchor;
      },
    } as unknown as Document;
    const urlRef = {
      createObjectURL: () => "blob:voxels-screenshot",
      revokeObjectURL: (url: string) => {
        revoked = url;
      },
    } as unknown as typeof URL;

    assert.equal(downloadBlob(new Blob(["png"]), "voxels x/1.png", documentRef, urlRef), true);
    assert.equal(clicked, true);
    assert.equal(anchor.download, "voxels_x_1.png");
    assert.equal(anchor.href, "blob:voxels-screenshot");
    await new Promise((resolve) => setTimeout(resolve, 1_010));
    assert.equal(revoked, "blob:voxels-screenshot");
  });

  it("rejects an empty capture before creating an object URL", () => {
    let created = false;
    const urlRef = {
      createObjectURL: () => {
        created = true;
        return "blob:unexpected";
      },
    } as unknown as typeof URL;
    assert.equal(downloadBlob(new Blob(), "empty.png", {} as Document, urlRef), false);
    assert.equal(created, false);
  });
});
