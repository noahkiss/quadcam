import { describe, expect, it } from "vitest";
import type { Device, Profile } from "../ipc/types";
import { addable, aircraftOf, modelStatus, modelText, withRadio } from "./radioAircraft";

const profile = (name: string, gear: Profile["gear"]): Profile => ({ name, aircraft: "", camera_make: "", camera_model: "", video_system: "Analog", keywords: [], author: "", place: null, edgetx_models: [], gear });

describe("radio aircraft", () => {
  it("says where the model is", () => {
    expect(modelStatus({ model: "model01.yml", checked: "card", found: true })).toBe("On the card");
    expect(modelStatus({ model: "model01.yml", checked: "latest backup", found: false })).toBe("Not in the latest backup");
    expect(modelStatus({ model: "model01.yml", checked: null, found: null })).toBe("Not checked: no backup yet");
    expect(modelStatus({ model: null, checked: "card", found: false })).toBe("No matching model on the card");
    expect(modelStatus({ model: null, checked: null, found: null })).toBe("No model named");
    expect(modelText({ model: "model01.yml", model_name: "WHOOP" })).toBe("WHOOP · model01.yml");
    expect(modelText({ model: null, model_name: null })).toBe("None");
  });

  it("reads the list from the record, else the saved list", () => {
    const listed = { profile: "A", model: null, model_name: null, checked: null, found: null, selected: false };
    const saved = [{ id: "r", kind: "radio", radio_aircraft: [listed] }] as unknown as Device[];
    expect(aircraftOf({ id: "r", kind: "radio", radio_aircraft: null } as unknown as Device, saved)).toEqual([listed]);
    expect(aircraftOf(null, saved)).toEqual([]);
  });

  it("adds and removes through the profile's gear", () => {
    const a = profile("A", { radio: "r", fc: "fc-1" });
    const b = profile("B", {});
    expect(addable([a, b], "r").map((p) => p.name)).toEqual(["B"]);
    expect(withRadio(b, "r")).toEqual({ radio: "r" });
    expect(withRadio(a, null)).toEqual({ fc: "fc-1" });
  });
});
