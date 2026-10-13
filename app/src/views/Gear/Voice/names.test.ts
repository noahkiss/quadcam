import { describe, expect, it } from "vitest";
import type { SetInfo, VoiceInfo } from "../../../ipc/types";
import { modelName, setsSummary, shortName, voiceName } from "./names";

describe("names", () => {
  it("makes a model's name from its id when the account has none", () => {
    expect(modelName("eleven_v4_turbo")).toBe("Eleven v4 Turbo");
    expect(modelName("eleven_flash_v2_5")).toBe("Eleven Flash v2.5");
    expect(modelName("eleven_multilingual_v2")).toBe("Eleven Multilingual v2");
    expect(modelName("kokoro")).toBe("kokoro");
    expect(modelName("eleven_v3", [{ id: "eleven_v3", name: "Eleven v3 (alpha)" } as never])).toBe("Eleven v3 (alpha)");
  });

  it("takes the short name before the dash", () => {
    expect(shortName("Xavier - Dominating, Metalic Announcer")).toBe("Xavier");
    expect(shortName("Callum")).toBe("Callum");
  });

  it("gives an old sample folder's name the account's spelling", () => {
    const voices = [{ id: "v-b", name: "Brittney - Social Media Voice - Fun, Youthful & Informative" }] as VoiceInfo[];
    expect(voiceName("Brittney - Social Media Voice - Fun, Youthful - Informative", "", voices)).toBe("Brittney - Social Media Voice - Fun, Youthful & Informative");
    expect(voiceName("Callum", "", voices)).toBe("Callum");
    expect(voiceName("Old", "v-b", voices)).toBe("Brittney - Social Media Voice - Fun, Youthful & Informative");
  });

  it("sums up a pack's sets in one line", () => {
    const set = (id: string, title: string, part_of: string | null = null) => ({ id, title, part_of }) as SetInfo;
    const sets = [set("edgetx", "Full EdgeTX English"), set("quad", "FPV quad", "edgetx"), set("heli", "Helicopter", "edgetx"), set("extras", "FPV extras"), set("easter", "Easter eggs"), set("quadcam", "QuadCam default"), set("sample", "Sample"), set("custom", "Your lines")];
    expect(setsSummary(["edgetx", "quad", "heli", "extras", "easter", "quadcam"], sets)).toBe("All sets");
    expect(setsSummary(["quad", "heli", "edgetx", "easter"], sets)).toBe("Full EdgeTX English + 1");
    expect(setsSummary(["easter"], sets)).toBe("Easter eggs");
    expect(setsSummary(["quad", "extras", "quadcam"], sets)).toBe("FPV quad + 2");
    expect(setsSummary([], sets)).toBe("Some lines");
  });
});
