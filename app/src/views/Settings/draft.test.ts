import { describe, expect, it } from "vitest";
import { gearDraft, gearValues } from "./draft";
import { gearDefaults } from "../../ipc/mock/gear";

describe("the blackbox settings in the Gear draft", () => {
  // The core's defaults are guarded in Rust (gear::tests::automations_are_off_but_backup);
  // here the input is spelled out, so only the mapping is under test.
  it("map off settings to off in the draft and back", () => {
    const off = { ...gearDefaults(), erase_blackbox: false, blackbox_msc: false, on_connect: { fc: ["backup"] } } as ReturnType<typeof gearDefaults>;
    const d = gearDraft(off);
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

describe("the firmware check in the Gear draft", () => {
  it("round-trips manual and daily", () => {
    const d = gearDraft({ ...gearDefaults(), firmware_check: "manual" });
    expect(d.firmwareDaily).toBe(false);
    expect(gearValues(d).firmwareCheck).toBe("manual");
    const daily = gearDraft({ ...gearDefaults(), firmware_check: "daily" });
    expect(daily.firmwareDaily).toBe(true);
    expect(gearValues(daily).firmwareCheck).toBe("daily");
  });
});
