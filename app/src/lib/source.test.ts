import { describe, expect, it } from "vitest";
import { fmtWhen } from "./backups";
import { sourceText } from "./source";

describe("sourceText", () => {
  it("names a backup by its time and keeps file names", () => {
    expect(sourceText(["fc-0a1b/2026-10-05T100000-before_apply dump all"], "Betaflight 4.5.1")).toBe(`From backup · ${fmtWhen("2026-10-05T10:00:00Z")} · Betaflight 4.5.1`);
    expect(sourceText(["quad.dump_all.txt", "apply.cli"], null)).toBe("quad.dump_all.txt, apply.cli");
    expect(sourceText([], null)).toBe("");
  });
});
