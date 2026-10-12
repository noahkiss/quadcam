import { describe, expect, it } from "vitest";
import type { SettingKind, StagedChange } from "../ipc/types";
import { numberRange, numberRaw, numberShown, openRadioChange, shownValue, stagedKeys, textError, toggleOn, toggleRaw } from "./radioSettings";
import { channelOrders } from "../ipc/mock/radio";

type Num = Extract<SettingKind, { type: "number" }>;
const volts: Num = { type: "number", min: 30, max: 120, unit: "V", mul: 1, decimals: 1, invert: null };
const secs: Num = { type: "number", min: 1, max: 120, unit: "s", mul: 5, decimals: 0, invert: null };
const bright: Num = { type: "number", min: 0, max: 95, unit: "", mul: 1, decimals: 0, invert: 100 };

describe("numbers", () => {
  it("show in the radio's units", () => {
    expect(numberShown(volts, "66")).toBe("6.6");
    expect(numberShown(secs, "6")).toBe("30");
    expect(numberShown(bright, "0")).toBe("100");
    expect(numberRange(bright)).toEqual(["5", "100"]);
    expect(numberRange(volts)).toEqual(["3.0", "12.0"]);
  });
  it("turn typed values back into file values, inside the range", () => {
    expect(numberRaw(volts, "7.2")).toEqual({ raw: "72" });
    expect(numberRaw(volts, "7,2")).toEqual({ raw: "72" });
    expect(numberRaw(secs, "45")).toEqual({ raw: "9" });
    expect(numberRaw(bright, "80")).toEqual({ raw: "20" });
    expect(numberRaw(volts, "13")).toEqual({ error: "Use a value from 3.0 to 12.0 V." });
    expect(numberRaw(secs, "42")).toEqual({ error: "Use steps of 5 s." });
    expect(numberRaw(bright, "2")).toEqual({ error: "Use a value from 5 to 100." });
    expect(numberRaw(volts, "")).toEqual({ error: "Type a number from 3.0 to 12.0 V." });
  });
});

describe("values as the radio shows them", () => {
  it("names choices, toggles and missing lines", () => {
    const toggle = { kind: { type: "toggle", invert: true } as SettingKind };
    expect(shownValue(toggle, "0")).toBe("On");
    expect(shownValue(toggle, "1")).toBe("Off");
    expect(shownValue({ kind: volts }, "66")).toBe("6.6 V");
    expect(shownValue({ kind: { type: "choice", choices: [{ value: "1", label: "Mode 2" }] } }, "1")).toBe("Mode 2");
    expect(shownValue({ kind: volts }, null)).toBe("Not on this card");
    const k = { type: "toggle" as const, invert: true };
    expect(toggleOn(k, "0")).toBe(true);
    expect(toggleRaw(k, false)).toBe("1");
  });
  it("checks owner IDs", () => {
    expect(textError(8, "QC-01")).toBeNull();
    expect(textError(8, "123456789")).toBe("Use 8 characters at most.");
    expect(textError(8, 'a"b')).toMatch(/letters, digits/);
  });
  it("lists the channel orders as EdgeTX does", () => {
    const o = channelOrders();
    expect(o).toHaveLength(24);
    expect(o[0]).toBe("RETA");
    expect(o[21]).toBe("AETR");
  });
});

describe("the staged change", () => {
  const change = (status: StagedChange["status"]): StagedChange =>
    ({ id: "c1", device: "r", title: "Radio settings", status, edits: [{ kind: "radio", ops: [{ op: "set_scalar", key: "beepVolume", value: "3" }, { op: "select_model", file: "model00.yml" }] }] }) as StagedChange;
  it("finds the open change and its keys", () => {
    expect(openRadioChange([change("ready")], "r")?.id).toBe("c1");
    expect(openRadioChange([change("verified")], "r")).toBeNull();
    expect([...stagedKeys(change("ready"))]).toEqual(["beepVolume", "currModel"]);
  });
});
