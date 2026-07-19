import { describe, expect, it } from "vite-plus/test";
import { ScenarioArguments } from "./arguments.ts";

describe("scenario arguments", () => {
  it("parses typed values and rejects leftovers", () => {
    const arguments_ = new ScenarioArguments([
      "--trace",
      "--mode=weather",
      "--viewport=1280x720",
      "--dpr=1.5",
    ]);
    expect(arguments_.flag("trace")).toBe(true);
    expect(arguments_.choice("mode", ["steady", "weather"], "steady")).toBe("weather");
    expect(arguments_.pair("viewport", { separator: "x", integer: true })).toEqual([1280, 720]);
    expect(arguments_.number("dpr", { minimum: 0.5, maximum: 4 })).toBe(1.5);
    arguments_.assertEmpty();
  });

  it("rejects duplicates, malformed values, and unknown options", () => {
    expect(() => new ScenarioArguments(["--mode=a", "--mode=b"])).toThrow(/duplicate/u);
    expect(() => new ScenarioArguments(["value"])).toThrow(/--name/u);
    const arguments_ = new ScenarioArguments(["--count=nope", "--extra"]);
    expect(() => arguments_.number("count")).toThrow(/finite number/u);
    expect(() => arguments_.assertEmpty()).toThrow(/--extra/u);
  });
});
