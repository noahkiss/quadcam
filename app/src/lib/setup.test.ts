import { describe, expect, it } from "vitest";
import mapFixture from "../../e2e/fixtures/switchmap.json";
import { switchMap } from "../ipc/normalize";
import type { JobRow, RadioSetup, RadioSetupDetail, SwitchMap } from "../ipc/types";
import type * as G from "../bindings";
import { defaultSetup } from "../ipc/mock/setup";
import { assign, atText, controlChoices, controlText, fit, generalProblems, learnBaseline, learnPick, learnResult, positionText, problemsFor, slotTypeText, sourceWords, statusText } from "./setup";

const map: SwitchMap = switchMap(mapFixture as unknown as G.SwitchMap);
const setup = () => defaultSetup("gx12") as RadioSetup;

/** CH1-CH8 at rest: every switch up (988 µs), sticks centred, throttle low. */
const rest = () => [1500, 1500, 988, 1500, 988, 988, 988, 988];

const row = (over: Partial<JobRow> = {}): JobRow => ({ job: "arm", label: "Arm", channel: "CH5", control: "SA", kind: "two_pos", high: 2, states: [], free_controls: ["SD"], ...over });

describe("words", () => {
  it("names positions as the radio shows them", () => {
    expect(positionText("SC2")).toBe("SC down");
    expect(positionText("SB1")).toBe("SB middle");
    expect(positionText("SH2", "momentary")).toBe("SH pressed");
    expect(positionText("TrimRudRight")).toBe("Rudder trim right");
    expect(atText("SA", 0)).toBe("SA up");
    expect(controlText("P1")).toBe("P1");
  });

  it("names fitted parts", () => {
    expect(slotTypeText("switch", "TOGGLE")).toBe("Momentary switch");
    expect(slotTypeText("switch", "2pos")).toBe("2-position switch");
    expect(slotTypeText("pot", "with_detent")).toBe("Pot with centre detent");
    expect(slotTypeText("pot", "none")).toBe("Not fitted");
    expect(slotTypeText("slider", "slider")).toBe("Slider");
  });

  it("says where the setup was read and how the radio stands", () => {
    expect(sourceWords("card")).toBe("From the card");
    expect(sourceWords("backup radio-1/2026-10-06T182000-manual (2026-10-06 18:20)")).toBe("From backup · 2026-10-06 18:20");
    const d = { problems: [], card_problem: null, files: [] } as unknown as RadioSetupDetail;
    expect(statusText(d)).toBe("The radio matches this setup.");
    expect(statusText({ ...d, files: ["MODELS/model01.yml", "RADIO/radio.yml"] })).toBe("2 files on the radio differ from this setup.");
    expect(statusText({ ...d, problems: [{ job: "arm", control: "SA", message: "x" }] })).toBe("Not saved: fix the problems below.");
  });
});

describe("the next setup", () => {
  const kinds = { SA: "two_pos", SD: "two_pos", P1: "pot", SH: "momentary" } as const;

  it("moves a job to a control, keeps job order, and drops the direction for a pot", () => {
    const s = assign(setup(), kinds, "arm", "SD", 0);
    expect(s.assignments.find((a) => a.job === "arm")).toEqual({ job: "arm", control: "SD", high: 0 });
    expect(s.assignments.map((a) => a.job)).toEqual(setup().assignments.map((a) => a.job));
    expect(assign(setup(), kinds, "vtx", "P1", 2).assignments.find((a) => a.job === "vtx")?.high).toBeNull();
    expect(assign(setup(), kinds, "arm", null, null).assignments.some((a) => a.job === "arm")).toBe(false);
  });

  it("records a fitted part, and the factory part takes the slot off the list", () => {
    const s = fit(setup(), "SA", "TOGGLE", "2POS");
    expect(s.fitted).toEqual([{ control: "SA", radio_type: "TOGGLE" }]);
    expect(fit(s, "SA", "2POS", "2POS").fitted).toEqual([]);
  });

  it("offers the job's control first, then the free ones", () => {
    expect(controlChoices(row())).toEqual(["SA", "SD"]);
    expect(controlChoices(row({ control: null }))).toEqual(["SD"]);
  });

  it("splits problems between rows and the page", () => {
    const p = [
      { job: "arm", control: "SA", message: "a" },
      { job: null, control: null, message: "b" },
    ];
    expect(problemsFor(p, "arm").map((x) => x.message)).toEqual(["a"]);
    expect(generalProblems(p).map((x) => x.message)).toEqual(["b"]);
  });
});

describe("Learn", () => {
  it("picks the switch that moved, at the end it rests on", () => {
    const base = learnBaseline(map, rest());
    expect(learnPick(map, base, rest())).toBeNull();
    // SB to the middle is not an end: nothing yet.
    const mid = rest();
    mid[5] = 1500;
    expect(learnPick(map, base, mid)).toBeNull();
    // SB down.
    const down = rest();
    down[5] = 2012;
    expect(learnPick(map, base, down)).toEqual({ control: "SB", high: 2 });
    // From SD down back up: SD, position 0.
    const sdDown = rest();
    sdDown[7] = 2012;
    expect(learnPick(map, learnBaseline(map, sdDown), rest())).toEqual({ control: "SD", high: 0 });
  });

  it("refuses a control that cannot do the job, and takes one that can", () => {
    const kinds = { SA: "two_pos", SB: "three_pos", SD: "two_pos" } as const;
    expect(learnResult(row(), { control: "SD", high: 0 }, kinds)).toEqual({ control: "SD", high: 0 });
    const r = learnResult(row(), { control: "SB", high: 2 }, kinds);
    expect("error" in r && r.error).toMatch(/SB moved, but it cannot do Arm/);
    expect("error" in learnResult(row(), { control: "SX", high: 2 }, kinds)).toBe(true);
  });
});
