import { describe, expect, it } from "vitest";
import mapFixture from "../../e2e/fixtures/switchmap.json";
import { switchMap } from "../ipc/normalize";
import type { SwitchMap } from "../ipc/types";
import type * as G from "../bindings";
import { asMode, live, MODE_LAYOUT, pct, sticksFrom } from "./controls";

const map: SwitchMap = switchMap(mapFixture as unknown as G.SwitchMap);

describe("stick modes", () => {
  it("puts throttle and yaw on the left in Mode 2", () => {
    expect(MODE_LAYOUT[2].left).toEqual({ v: "throttle", h: "yaw" });
    expect(MODE_LAYOUT[2].right).toEqual({ v: "pitch", h: "roll" });
    expect(MODE_LAYOUT[1].right.v).toBe("throttle");
    // Every mode places each stick axis once.
    for (const m of [1, 2, 3, 4] as const) {
      const l = MODE_LAYOUT[m];
      expect(new Set([l.left.v, l.left.h, l.right.v, l.right.h]).size).toBe(4);
    }
  });
  it("reads a bad saved mode as Mode 2", () => {
    expect(asMode(3)).toBe(3);
    expect(asMode(7)).toBe(2);
    expect(asMode(undefined)).toBe(2);
  });
});

describe("sticks from channels", () => {
  it("reads AETR channels, with the model's weights", () => {
    const v = sticksFrom([2012, 1500, 988, 1244]);
    expect(v).toEqual({ roll: 1, pitch: 0, throttle: -1, yaw: -0.5 });
    // A reversed, half-weight mix still reads full travel.
    expect(sticksFrom([1244], [{ stick: "roll", ch: 1, weight: -50 }]).roll).toBe(1);
    expect(pct(0.5)).toBe("+50%");
    expect(pct(-1)).toBe("-100%");
  });
});

describe("live matching", () => {
  it("marks each control's position and the modes on, as the core does", () => {
    // The core's own test (tests/switchmap.rs, whoop): SA down, SB mid, SC down, SD up.
    const l = live(map, "radio", [1500, 1500, 988, 1500, 2012, 1500, 2012, 988]);
    expect(l.positions).toMatchObject({ SA: 1, SB: 1, SC: 2, SD: 0, SE: null });
    expect(l.modes).toEqual(["ARM", "HORIZON"]);
    expect(l.adjustments).toEqual(["Rate profile 3"]);
    // Between positions: no match.
    expect(live(map, "radio", [1500, 1500, 988, 1500, 1250]).positions.SA).toBeNull();
  });

  it("matches a position that only shows with another control moved, as the core does", () => {
    const m = structuredClone(map);
    // SE has no channels of its own; with SD down it sends CH9.
    const se = m.rows.find((r) => r.id === "SE")!;
    se.positions[1].combos = [{ with: ["SD down"], channels: [{ ch: 9, us: 2012 }], fc: [], radio: [] }];
    expect(live(m, "radio", [1500, 1500, 988, 1500, 2012, 1500, 2012, 2012, 2012]).positions.SE).toBe(1);
    expect(live(m, "radio", [1500, 1500, 988, 1500, 2012, 1500, 2012, 2012, 988]).positions.SE).toBeNull();
  });
});
