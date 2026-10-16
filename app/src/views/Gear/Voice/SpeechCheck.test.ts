import { describe, expect, it } from "vitest";
import type { VoicePack } from "../../../ipc/types";
import { checkRows, isStudio, needsReview } from "./SpeechCheck";

const heard = (path: string, ok: boolean, accepted = false) => ({ path, text: path, heard: ok ? path : "Battery.", ok, accepted, confidence: 0.9 });

const pack = (k: Partial<VoicePack>) => ({ id: "local-elevenlabs-x", voice: "Callum", provider: "elevenlabs", local: true, installed: true, ...k }) as VoicePack;

describe("checkRows", () => {
  it("puts the lines heard wrong, the re-takes and the flags first", () => {
    const rows = checkRows(
      pack({
        heard: [heard("a", true), heard("b", false), heard("c", false, true), heard("d", true)],
        retakes: [
          { path: "b", text: "b", reason: 'The speech check heard "Battery.".' },
          { path: "z", text: "z", reason: "the cut is silent" },
        ],
        flags: [{ line: "d", reason: "noise", note: "", at: "", retakes: 0 }],
      }),
    );
    expect(rows.map((r) => r.path)).toEqual(["b", "d", "z", "a", "c"]);
    expect(rows.filter(needsReview).map((r) => r.path)).toEqual(["b", "d", "z"]);
    // A re-take the check never heard has no heard line, only its reason.
    expect(rows.find((r) => r.path === "z")).toMatchObject({ heard: null, retake: "the cut is silent" });
  });

  it("an accepted line needs no review", () => {
    expect(checkRows(pack({ heard: [heard("c", false, true)] })).filter(needsReview)).toEqual([]);
  });
});

describe("isStudio", () => {
  it("is a pack the Voice studio rendered here", () => {
    expect(isStudio(pack({}))).toBe(true);
    expect(isStudio(pack({ local: false }))).toBe(false);
    expect(isStudio(pack({ provider: "say" }))).toBe(false);
  });
});
