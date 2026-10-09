import { describe, expect, it } from "vitest";
import type { CalloutView, ModelTimer } from "../ipc/types";
import { calloutOp, checklistText, formOf, freeTimer, nextScreen, overWidth, parseChecklist, parseSources, repeatText, timerField, whenText } from "./models";
import { mergeOps, opKey } from "../ipc/mock/model";

const timer = (index: number): ModelTimer => ({ index, name: "", swtch: "NONE", mode: "ON", value: 0, persistent: "0", fields: [{ key: "name", value: '"FLT"' }, { key: "minuteBeep", value: "1" }] });

describe("timers and screens", () => {
  it("reads raw fields unquoted and finds a free slot", () => {
    expect(timerField(timer(0), "name")).toBe("FLT");
    expect(timerField(timer(0), "minuteBeep")).toBe("1");
    expect(timerField(timer(0), "missing")).toBe("");
    expect(freeTimer([timer(0), timer(2)])).toBe(1);
    expect(freeTimer([timer(0), timer(1), timer(2)])).toBeNull();
  });
  it("adds screens in order, to 4", () => {
    expect(nextScreen([])).toBe(0);
    expect(nextScreen([0, 1])).toBe(2);
    expect(nextScreen([0, 1, 2, 3])).toBeNull();
  });
  it("splits sources on commas", () => {
    expect(parseSources(" {RxBt} , Tmr1,, ")).toEqual(["{RxBt}", "Tmr1"]);
    expect(parseSources("")).toEqual([]);
  });
});

describe("callouts", () => {
  const below: CalloutView = { track: "lowbat", swtch: "L3", repeat: "5", when: { when: "below", source: "{RxBt}", value: "3.5", delay_ds: 20 } };
  it("says when and how often", () => {
    expect(whenText(below)).toBe("RxBt below 3.5 for 2 s");
    expect(whenText({ track: "armed", swtch: "L1", repeat: "1x", when: { when: "switch", swtch: "L1" } })).toBe("while L1 is on");
    expect(whenText({ track: "x", swtch: "L9", repeat: "1x", when: null })).toBe("on L9 (edit in EdgeTX)");
    expect([repeatText("1x"), repeatText("!1x"), repeatText("5")]).toEqual(["once", "once, not at power-on", "every 5 s"]);
  });
  it("round-trips a callout through the form", () => {
    const f = formOf(below, ["RxBt", "Capa"]);
    expect(f).toMatchObject({ track: "lowbat", kind: "below", source: "{RxBt}", value: "3.5", delay: "2", repeat: "every", every: "5" });
    expect(calloutOp(f)).toEqual({ op: { op: "set_callout", callout: { track: "lowbat", when: "below", source: "{RxBt}", value: "3.5", delay_ds: 20, repeat: "5" } } });
  });
  it("starts a new callout on the battery sensor", () => {
    expect(formOf(null, ["1RSS", "RxBt"])).toMatchObject({ kind: "below", source: "{RxBt}", repeat: "1x" });
    expect(formOf(null, ["Capa"]).source).toBe("{Capa}");
  });
  it("refuses a form that cannot stage", () => {
    const f = formOf(null, ["RxBt"]);
    expect(calloutOp(f)).toEqual({ error: "Pick a sound for the callout." });
    expect(calloutOp({ ...f, track: "lowbat" })).toEqual({ error: "The value is a number." });
    expect(calloutOp({ ...f, track: "lowbat", value: "3.5", repeat: "every", every: "x" })).toEqual({ error: "Seconds between repeats is a whole number." });
    expect(calloutOp({ ...f, track: "armed", kind: "switch", swtch: " " })).toEqual({ error: "Name the switch." });
    expect(calloutOp({ ...f, track: "armed", kind: "switch", swtch: "SA2", repeat: "!1x" })).toEqual({ op: { op: "set_callout", callout: { track: "armed", when: "switch", swtch: "SA2", repeat: "!1x" } } });
  });
});

describe("checklist", () => {
  it("reads ticks and writes them back", () => {
    const items = parseChecklist("=Props tight\r\nBattery strapped\n\n");
    expect(items).toEqual([{ tick: true, text: "Props tight" }, { tick: false, text: "Battery strapped" }]);
    expect(checklistText(items)).toBe("=Props tight\nBattery strapped\n");
    expect(checklistText([])).toBe("");
    expect(parseChecklist(null)).toEqual([]);
  });
  it("counts the tick box against the screen width", () => {
    const items = [{ tick: true, text: "12345678901234567890" }, { tick: false, text: "12345678901234567890" }];
    expect(overWidth(items, 20)).toBe(1);
    expect(overWidth(items, null)).toBe(0);
  });
});

describe("staged ops", () => {
  it("keys an op by the setting it is about", () => {
    expect(opKey({ op: "set_timer", index: 1, fields: [] })).toBe("timer:1");
    expect(opKey({ op: "remove_timer", index: 1 })).toBe("timer:1");
    expect(opKey({ op: "remove_callout", track: "lowbat" })).toBe(opKey({ op: "set_callout", callout: { track: "lowbat", when: "switch", swtch: "SA2" } }));
  });
  it("lets the last word win and merges timer fields and sensor logs", () => {
    const merged = mergeOps(
      [{ op: "set_timer", index: 1, fields: [{ key: "name", value: "A" }] }, { op: "set_sensor_logs", sensors: [{ label: "RxBt", logs: false }] }],
      [{ op: "set_timer", index: 1, fields: [{ key: "name", value: "B" }, { key: "mode", value: "ON" }] }, { op: "set_sensor_logs", sensors: [{ label: "Capa", logs: false }, { label: "RxBt", logs: true }] }, { op: "set_rf_alarms", warning: 50, critical: 40 }],
    );
    expect(merged).toEqual([
      { op: "set_timer", index: 1, fields: [{ key: "name", value: "B" }, { key: "mode", value: "ON" }] },
      { op: "set_sensor_logs", sensors: [{ label: "RxBt", logs: true }, { label: "Capa", logs: false }] },
      { op: "set_rf_alarms", warning: 50, critical: 40 },
    ]);
  });
});
