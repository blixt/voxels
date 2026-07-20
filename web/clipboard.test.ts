import { describe, expect, it } from "vite-plus/test";
import { writeClipboardText } from "./clipboard.ts";

describe("World Lab clipboard", () => {
  it("writes the complete report and confirms success", async () => {
    const writes: string[] = [];
    const copied = await writeClipboardText(
      {
        writeText: async (text) => {
          writes.push(text);
        },
      },
      "VOXELS / WORLD LAB\nEye position (m): X 1.000, Y 2.000, Z 3.000",
    );

    expect(copied).toBe(true);
    expect(writes).toEqual(["VOXELS / WORLD LAB\nEye position (m): X 1.000, Y 2.000, Z 3.000"]);
  });

  it("reports unavailable and rejected clipboard writes", async () => {
    await expect(writeClipboardText(undefined, "report")).resolves.toBe(false);
    await expect(
      writeClipboardText(
        {
          writeText: async () => {
            throw new Error("permission denied");
          },
        },
        "report",
      ),
    ).resolves.toBe(false);
  });
});
