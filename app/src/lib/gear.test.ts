import { describe, expect, it } from "vitest";
import type { Connected } from "../ipc/types";
import { deviceName, deviceState, linkHandle, pageKey, worst } from "./gear";

const card = (whole: string | null): Connected => ({ id: "dvr-1", kind: "dvr_card", link: { kind: "volume", mount: "/Volumes/DVR", whole_disk: whole }, identity: {} });
const x = (over: Partial<Parameters<typeof deviceState>[2]> = {}) => ({ status: { working: [], reminders: [] }, unmountedSince: {}, graceS: 60, now: 100_000, ...over });

describe("linkHandle", () => {
  // The same keys as `core::link_handle`, which holds and reminders use.
  it("is the whole disk, else the mount, the port, or the DFU ids", () => {
    expect(linkHandle(card("disk4").link)).toBe("disk4");
    expect(linkHandle(card(null).link)).toBe("/Volumes/DVR");
    expect(linkHandle({ kind: "serial", port: "/dev/cu.usbmodem1", vid: 0x0483, pid: 0x5740 })).toBe("/dev/cu.usbmodem1");
    expect(linkHandle({ kind: "dfu", vid: 0x0483, pid: 0xdf11 })).toBe("dfu-0483:df11");
  });
});

describe("deviceState", () => {
  const saved = { ...card("disk4"), device: { id: "dvr-1", kind: "dvr_card" as const, name: "DVR" } };
  it("is working while a job holds the link", () => {
    expect(deviceState(saved, false, x({ status: { working: ["disk4"], reminders: [] } }))).toBe("working");
  });
  it("needs attention for a device that is not saved", () => {
    expect(deviceState(card("disk4"), false, x())).toBe("attention");
    expect(deviceState(saved, false, x())).toBe("connected");
  });
  it("is safe to unplug once unmounted, and still inserted when the reminder is due", () => {
    const since = { disk4: 100_000 - 30_000 };
    expect(deviceState(saved, true, x({ unmountedSince: since }))).toBe("safe");
    expect(deviceState(saved, true, x({ unmountedSince: since, status: { working: [], reminders: ["disk4"] } }))).toBe("safe");
    expect(deviceState(saved, true, x({ unmountedSince: { disk4: 0 }, status: { working: [], reminders: ["disk4"] } }))).toBe("inserted");
  });
  it("ranks states by urgency", () => {
    expect(worst("connected", "safe")).toBe("safe");
    expect(worst("working", "attention")).toBe("attention");
    expect(worst("inserted", "working")).toBe("inserted");
  });
});

describe("names", () => {
  it("names an unnamed device by its kind, and keys a page by id or link", () => {
    expect(deviceName({ name: " ", kind: "fc" })).toBe("Unnamed FC");
    expect(pageKey(card("disk4"))).toBe("dvr-1");
    expect(pageKey({ ...card("disk4"), id: null })).toBe("link:disk4");
  });
});
