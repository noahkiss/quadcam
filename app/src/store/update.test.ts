import { describe, expect, it } from "vitest";
import type { UpdateStatus } from "../ipc/types";
import { updateBanner, updateLine, updateOffer } from "./update";

const u = (o: Partial<UpdateStatus>): UpdateStatus => ({ phase: "idle", current: "0.13.0", version: null, notes_url: null, done: 0, total: null, error: null, checked_at: null, waiting_for: null, ...o });

describe("update banner", () => {
  it("shows a found update until Later hides that version", () => {
    const found = u({ phase: "available", version: "0.14.0" });
    expect(updateBanner({ update: found, updateLater: null })).toBe(found);
    expect(updateBanner({ update: found, updateLater: "0.14.0" })).toBeNull();
    expect(updateBanner({ update: { ...found, version: "0.15.0" }, updateLater: "0.14.0" })).not.toBeNull();
  });
  it("stays away when nothing is newer or a check failed", () => {
    expect(updateBanner({ update: u({ phase: "current" }), updateLater: null })).toBeNull();
    expect(updateBanner({ update: u({ phase: "failed", error: "offline" }), updateLater: null })).toBeNull();
    expect(updateBanner({ update: null, updateLater: null })).toBeNull();
  });
  it("words each phase for Settings", () => {
    expect(updateLine(u({}))).toBe("QuadCam 0.13.0. Not checked yet.");
    expect(updateLine(u({ phase: "current" }))).toBe("QuadCam 0.13.0 is the newest version.");
    expect(updateLine(u({ phase: "failed", error: "offline" }))).toBe("offline");
    expect(updateLine(u({ phase: "downloading", version: "0.14.0", done: 1, total: 4 }))).toBe("Downloading QuadCam 0.14.0… 25%");
    expect(updateLine(u({ phase: "downloading", version: "0.14.0" }))).toBe("Downloading QuadCam 0.14.0…");
    expect(updateLine(u({ phase: "ready", version: "0.14.0", waiting_for: "the device job" }))).toBe("QuadCam 0.14.0 is ready. It can restart after the device job ends.");
  });
  it("Settings offers a found update even after Later", () => {
    const found = u({ phase: "available", version: "0.14.0" });
    expect(updateOffer(found)).toBe(found);
    expect(updateOffer(u({ phase: "failed", version: "0.14.0", error: "reset" }))).not.toBeNull();
    expect(updateOffer(u({ phase: "current" }))).toBeNull();
    expect(updateOffer(u({ phase: "failed", error: "offline" }))).toBeNull();
  });
});
