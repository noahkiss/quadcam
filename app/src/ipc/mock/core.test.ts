import { describe, expect, it } from "vitest";
import { MockCore, type Scenario } from "./core";

const make = (scenario?: Scenario) => {
  const events: string[] = [];
  return { core: new MockCore((e) => events.push(e), { scenario }), events };
};

describe("MockCore", () => {
  it("rates and flags, and says the library changed", () => {
    const { core, events } = make();
    const [c] = core.rate(["xd0d144c9ce319e86"], 5, "reject");
    expect(c).toMatchObject({ rating: 5, flag: "reject" });
    expect(events).toContain("library-changed");
    expect(() => core.rate(["xd0d144c9ce319e86"], 6, null)).toThrow();
  });

  it("asks before dropping an exported cut", () => {
    const { core } = make();
    expect(core.libraryCuts("xd0d144c9ce319e86", [], null)).toMatchObject({ status: "confirm" });
    expect(core.libraryCuts("xd0d144c9ce319e86", [], "trash")).toMatchObject({ status: "applied", trashed: [expect.stringContaining("_cut1.mp4")] });
  });

  it("puts trashed clips back", () => {
    const { core } = make();
    const r = core.trashClips(["x07fee4b870d01a6f"]);
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

  it("replaces a clip that is already in the library, as the core does", () => {
    const { core } = make("review");
    const before = core.libraryView().clips.map((c) => c.id);
    core.importClips({ output_dir: "/Users/pilot/Movies/quadcam", format: "mp4" });
    const ids = core.libraryView().clips.map((c) => c.id);
    expect(before).toContain("x07fee4b870d01a6f");
    expect(new Set(ids).size).toBe(ids.length);
  });

  it("stages a voice on several radios and deletes a pack", () => {
    const { core } = make();
    core.gear.devices.push({ ...structuredClone(core.gear.devices[0]), id: "radio-two", name: "Bench radio" });
    core.gear.devices.push({ ...structuredClone(core.gear.devices[0]), id: "radio-ethos", name: "Other radio", identity: { firmware: "ETHOS" } });
    core.dispatch("gear_voice_render", { voice: "", lines: [], dry_run: false, confirm: false });
    expect(() => core.dispatch("gear_voice_choose_radios", { pack: "local-say-samantha", radios: ["radio-ethos"] })).toThrow(/EdgeTX radios/);
    const r = core.dispatch("gear_voice_choose_radios", { pack: "local-say-samantha", all: true }) as { staged: { device: string }[] };
    expect(r.staged.map((c) => c.device)).toEqual(["radio-1f2e3d4c5b6a7980", "radio-two"]);
    const v = core.dispatch("gear_voice", { radio: null }) as { packs: { id: string; radios: string[] }[] };
    expect(v.packs.find((k) => k.id === "local-say-samantha")?.radios).toHaveLength(2);
    expect(core.dispatch("gear_voice_pack_delete", { pack: "local-say-samantha", takes: false })).toMatchObject({ takes: 0, radios: ["radio-1f2e3d4c5b6a7980", "radio-two"] });
    expect(() => core.dispatch("gear_voice_pack_delete", { pack: "local-say-samantha" })).toThrow(/not installed/);
  });
});
