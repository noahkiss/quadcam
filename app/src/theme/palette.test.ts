import { execFileSync } from "node:child_process";
import { describe, expect, it } from "vitest";

describe("palette.css", () => {
  it("matches @catppuccin/palette", () => {
    expect(() => execFileSync("node", ["scripts/gen-palette.mjs", "--check"], { stdio: "pipe" })).not.toThrow();
  });
});
