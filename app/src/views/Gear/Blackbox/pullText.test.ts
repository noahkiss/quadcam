import { describe, expect, it } from "vitest";
import { pullText } from "./BlackboxSegment";
import type { BlackboxPullResult } from "../../../ipc/types";
import { pull } from "../../../ipc/mock/blackbox";
import { freshBlackbox } from "../../../ipc/mock/blackbox";

const fc = [{ id: "fc-1", kind: "fc" as const, link: { kind: "serial" as const, port: "/dev/cu.x", vid: 1, pid: 2, serial_number: null, manufacturer: null, product: null }, identity: {} }];

describe("pullText", () => {
  it("says what a pull did about the erase", () => {
    const b = freshBlackbox();
    const kept = pull(b, fc, {}, false, new Date()).result;
    expect(pullText(kept)).toBe("3 logs stored (4 MB). Flash kept.");
    const erased = pull(freshBlackbox(), fc, { gearEraseBlackbox: true }, false, new Date()).result;
    expect(pullText(erased)).toBe("3 logs stored (4 MB). Flash erased. Safe to unplug.");
    const skipped: BlackboxPullResult = { ...kept, erase: "skipped", erase_note: "Not erased: the USB timer is short." };
    expect(pullText(skipped)).toContain("Not erased: the USB timer is short.");
    expect(pullText({ ...kept, pull: null })).toBe("The blackbox flash is empty.");
  });

  it("keep overrides the setting for one run", () => {
    const r = pull(freshBlackbox(), fc, { gearEraseBlackbox: true }, true, new Date()).result;
    expect(r.erase).toBe("off");
  });
});
