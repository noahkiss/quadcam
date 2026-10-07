import { describe, expect, it } from "vitest";
import { controlText, endName, stepText, sticksOf } from "./simcal";

describe("sim calibration text", () => {
  it("names ends by direction, following Reverse", () => {
    expect(endName("pitch", "low", false)).toBe("Back end");
    expect(endName("pitch", "low", true)).toBe("Forward end");
    expect(endName("throttle", "high", true)).toBe("Bottom");
    expect(endName("roll", "high", false)).toBe("Right end");
  });
  it("says what a control is", () => {
    expect(controlText({ kind: "channel", ch: 5, min_us: 1700, max_us: 2100 })).toBe("CH5 1700-2100 µs");
    expect(controlText({ kind: "button", button: 1, pressed: true })).toBe("Button 1");
    expect(controlText(null)).toBe("None");
    expect(stepText("capture", false, "turtle")).toBe("Move the control for Turtle.");
  });
  it("maps the output to the widget", () => {
    const v = { sticks: { roll: 100, pitch: -50, yaw: 0, throttle: 0 } } as Parameters<typeof sticksOf>[0];
    expect(sticksOf(v)).toEqual({ roll: 1, pitch: -0.5, yaw: 0, throttle: -1 });
  });
});
