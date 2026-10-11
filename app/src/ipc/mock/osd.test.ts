import { describe, expect, it } from "vitest";
import { withEdits } from "./osd";
import * as seed from "./seed";
import type { Edit } from "../types";

describe("the mock's OSD check", () => {
  it("redraws the recorded real-core view cell for cell", () => {
    const view = seed.osd();
    // An edit that moves nothing makes the mock redraw and check every profile itself.
    const vbat = view.elements.find((e) => e.name === "vbat")!;
    const same: Edit = { kind: "osd_element", element: "vbat", x: vbat.x, y: vbat.y, profiles: vbat.profiles };
    const redrawn = withEdits(view, [same]);
    redrawn.profiles.forEach((p, i) => {
      const core = view.profiles[i];
      expect(p.rows).toEqual(core.rows);
      expect(p.boxes).toEqual(core.boxes);
      expect(p.problems).toEqual(core.problems);
      expect(p.notes).toEqual(core.notes);
    });
  });

  it("reports the horizon's pitch sweep off screen, as the core does", () => {
    const view = seed.osd();
    const ah = view.elements.find((e) => e.name === "ah")!;
    // At y 8 the level line (row 12) is on the 16-row screen; the sweep (rows 8-17) is not.
    const low: Edit = { kind: "osd_element", element: "ah", x: ah.x, y: 8, profiles: ah.profiles };
    const p = withEdits(view, [low]).profiles.find((x) => x.index === ah.profiles[0])!;
    expect(p.problems).toContainEqual(expect.objectContaining({ kind: "off_screen", element: "ah", cells: 0 }));
    expect(p.problems.find((x) => x.element === "ah")!.message).toContain("rows 8-17");
  });
});
