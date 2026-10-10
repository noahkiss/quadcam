import { describe, expect, it } from "vitest";
import { gearDraft, gearValues } from "./draft";
import { gearDefaults } from "../../ipc/mock/gear";

describe("the blackbox settings in the Gear draft", () => {
  it("are off by default, as in the core", () => {
    const d = gearDraft(gearDefaults());
    expect(d.eraseBlackbox).toBe(false);
    expect(d.blackboxMsc).toBe(false);
    expect(d.onConnect.fc).toEqual(["backup"]);
    const v = gearValues(d);
    expect(v.gearEraseBlackbox).toBe(false);
    expect(v.gearBlackboxMsc).toBe(false);
  });

  it("round-trip through the draft and keep the step order", () => {
    const g = { ...gearDefaults(), erase_blackbox: true, on_connect: { fc: ["blackbox", "backup"] } } as ReturnType<typeof gearDefaults>;
    const d = gearDraft(g);
    expect(d.eraseBlackbox).toBe(true);
    expect(d.onConnect.fc).toEqual(["backup", "blackbox"]);
    expect(gearValues(d).gearOnConnect).toMatchObject({ fc: ["backup", "blackbox"] });
  });
});
