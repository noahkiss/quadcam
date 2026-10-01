import { describe, expect, it } from "vitest";
import { MockCore, type Scenario } from "./core";

const make = (scenario?: Scenario) => {
  const events: string[] = [];
  return { core: new MockCore((e) => events.push(e), { scenario }), events };
};

describe("MockCore", () => {
  it("rates and flags, and says the library changed", () => {
    const { core, events } = make();
    const [c] = core.rate(["dce3e50d5303bc2b"], 5, "reject");
    expect(c).toMatchObject({ rating: 5, flag: "reject" });
    expect(events).toContain("library-changed");
    expect(() => core.rate(["dce3e50d5303bc2b"], 6, null)).toThrow();
  });

  it("asks before dropping an exported cut", () => {
    const { core } = make();
    expect(core.libraryCuts("dce3e50d5303bc2b", [], null)).toMatchObject({ status: "confirm" });
    expect(core.libraryCuts("dce3e50d5303bc2b", [], "trash")).toMatchObject({ status: "applied", trashed: [expect.stringContaining("_cut1.mp4")] });
  });

  it("puts trashed clips back", () => {
    const { core } = make();
    const r = core.trashClips(["e86ab59aced6fd18"]);
    expect(core.libraryView().clips).toHaveLength(2);
    core.untrash(r.moved);
    expect(core.libraryView().clips).toHaveLength(3);
  });

  it("imports the plans that are not skipped", () => {
    const { core } = make("review");
    core.patch([{ id: 0, name: "loops" }]);
    const out = core.importClips({ output_dir: "/Users/pilot/Movies/quadcam", format: "mp4" });
    expect(out.summary.imported).toBe(4);
    expect(core.libraryView().clips.some((c) => c.name === "loops")).toBe(true);
  });
});
