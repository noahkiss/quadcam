import { describe, expect, it } from "vitest";
import { errText } from "./api";

describe("errText", () => {
  it("passes plain errors through", () => {
    expect(errText("No card is plugged in.")).toBe("No card is plugged in.");
    expect(errText(new Error("boom"))).toBe("boom");
    expect(errText(42)).toBe("42");
  });

  it("shows a refusal as its reason, without the code", () => {
    expect(errText("Refused (usb_heat): This FC has run on USB with its battery in past its limit. Unplug the battery and let it cool.")).toBe(
      "This FC has run on USB with its battery in past its limit. Unplug the battery and let it cool.",
    );
    expect(errText(new Error("Refused (port_busy): /dev/cu.x is open in another QuadCam window or the CLI."))).toBe("/dev/cu.x is open in another QuadCam window or the CLI.");
  });

  it("drops the code of a refusal behind a context", () => {
    expect(errText("flashing: Refused (no_backup): Back up this radio first.")).toBe("flashing: Back up this radio first.");
  });

  it("keeps a refusal that has no code", () => {
    expect(errText("Refused: format needs an explicit confirm.")).toBe("Refused: format needs an explicit confirm.");
  });
});
