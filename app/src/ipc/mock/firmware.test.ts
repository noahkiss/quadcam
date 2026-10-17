import { describe, expect, it } from "vitest";
import { freshFirmware, view } from "./firmware";
import { buildDay, installedLabel, newestLabel } from "../../views/Gear/Firmware/FirmwarePage";
import type { Device } from "../types";

const NOW = "2026-10-09T12:00:00Z";
const meteor = (build: string, more: Partial<Device> = {}): Device => ({
  id: "fc-meteor",
  kind: "fc",
  name: "Meteor",
  aircraft: "Meteor75 Pro II",
  identity: { board: "BETAFPVG473", firmware: "Betaflight", version: "2025.12.5-alpha", build, manufacturer_id: "BEFH" },
  last_seen: null,
  last_backup: null,
  ...more,
});

describe("the mock's vendor channel", () => {
  it("checks an FC on its maker's build against the maker's file", () => {
    const st = freshFirmware();
    const row = view(st, [meteor("Jun 25 2026 03:24:50 eb2bb5a")], true, NOW).devices[0];
    expect(row).toMatchObject({ channel: "vendor", channel_reason: "local_build", vendor: "BetaFPV", state: "up_to_date", latest: "2025.12.5-alpha", official: "2026.6.0", flashable: false });
    expect(installedLabel(row)).toBe("2025.12.5-alpha (BetaFPV, Jun 25 2026)");
    expect(newestLabel(row)).toBe("2025.12.5-alpha (BetaFPV, Jun 25 2026)");
  });

  it("has an update when the maker's file is newer, and follows the person's choice", () => {
    const st = freshFirmware();
    const old = meteor("Mar 2 2026 10:00:00 1234567", { identity: { board: "BETAFPVG473", version: "2025.12.3", build: "Mar 2 2026 10:00:00 1234567" } });
    let row = view(st, [old], true, NOW).devices[0];
    expect(row).toMatchObject({ channel: "vendor", state: "update", note: "QuadCam checks BetaFPV builds. It does not flash them." });
    row = view(st, [{ ...old, firmware_channel: "official" }], false, NOW).devices[0];
    expect(row).toMatchObject({ channel: "official", channel_reason: "chosen", latest: "2026.6.0", state: "update" });
    expect(installedLabel(row)).toBe("2025.12.3");
  });

  it("calls a cloud build of another day official, with a note", () => {
    const st = freshFirmware();
    st.builds.k1 = "2026-07-30";
    const row = view(st, [meteor("Jul 30 2026 01:00:00 abcdef1", { identity: { board: "BETAFPVG473", version: "2025.12.5-alpha", build: "Jul 30 2026 01:00:00 abcdef1", build_key: "k1" } })], true, NOW).devices[0];
    expect(row).toMatchObject({ channel: "official", channel_reason: "cloud_build", channel_note: "This FC runs a Betaflight cloud build, not BetaFPV's file." });
  });

  it("leaves an FC without a maker row official", () => {
    const row = view(freshFirmware(), [meteor("x", { aircraft: "Whoop" })], true, NOW).devices[0];
    expect(row).toMatchObject({ channel: "official", channel_reason: "no_source", vendor: null });
  });

  it("prints a build day as Betaflight does", () => {
    expect(buildDay("2026-06-05")).toBe("Jun 5 2026");
    expect(buildDay(null)).toBeNull();
  });
});
