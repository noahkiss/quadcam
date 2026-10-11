import { describe, expect, it } from "vitest";
import type { UpdateStatus } from "../ipc/types";
import { updateBanner, updateLine } from "./update";

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
  });
});
