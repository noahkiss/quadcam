// The gallery's player: a queue plays in order with the current take marked, a new play stops
// the one before it, and Stop ends the queue.
import { act, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { usePlayer, type Track } from "./usePlayer";

vi.mock("../../../ipc/api", () => ({ fileSrc: (p: string) => `asset://${p}` }));
const toast = vi.hoisted(() => vi.fn());
vi.mock("../../../components/toastStore", () => ({ toast }));

class FakeAudio {
  static made: FakeAudio[] = [];
  src: string;
  paused = false;
  onended: (() => void) | null = null;
  onerror: (() => void) | null = null;
  constructor(src: string) {
    this.src = src;
    FakeAudio.made.push(this);
  }
  /** A take the web view refuses: play() rejects, as for a src outside the asset scope. */
  static refuse = false;
  play() {
    return FakeAudio.refuse ? Promise.reject(new Error("NotSupportedError")) : Promise.resolve();
  }
  pause() {
    this.paused = true;
  }
}

const tracks: Track[] = ["a", "b", "c"].map((l) => ({ card: "x", line: l, file: `/s/${l}.wav` }));

describe("usePlayer", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    FakeAudio.made = [];
    FakeAudio.refuse = false;
    toast.mockClear();
    vi.stubGlobal("Audio", FakeAudio);
  });
  afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

  it("plays a queue in order, then clears", () => {
    const { result } = renderHook(() => usePlayer());
    act(() => result.current.playAll(tracks, "x"));
    expect(result.current.now?.line).toBe("a");
    expect(result.current.queue).toBe("x");
    act(() => {
      FakeAudio.made[0].onended?.();
      // A second end of the same take is ignored.
      FakeAudio.made[0].onended?.();
      vi.runAllTimers();
    });
    expect(result.current.now?.line).toBe("b");
    expect(FakeAudio.made).toHaveLength(2);
    act(() => {
      FakeAudio.made[1].onended?.();
      vi.runAllTimers();
      FakeAudio.made[2].onended?.();
      vi.runAllTimers();
    });
    expect(result.current.now).toBeNull();
    expect(result.current.queue).toBeNull();
  });

  it("stops the queue on Stop and on a new play", () => {
    const { result } = renderHook(() => usePlayer());
    act(() => result.current.playAll(tracks, "x"));
    act(() => result.current.play(tracks[2]));
    expect(FakeAudio.made[0].paused).toBe(true);
    expect(result.current.queue).toBeNull();
    expect(result.current.now?.line).toBe("c");
    act(() => result.current.stop());
    expect(FakeAudio.made[1].paused).toBe(true);
    expect(result.current.now).toBeNull();
  });

  it("names a take that fails in one toast per run, and goes on to the next", async () => {
    FakeAudio.refuse = true;
    const { result } = renderHook(() => usePlayer());
    act(() => result.current.playAll(tracks, "x"));
    for (let i = 0; i < 3; i++) {
      await act(async () => {
        // The error event and the play() rejection report one failure.
        FakeAudio.made[i].onerror?.();
        await Promise.resolve();
        vi.runAllTimers();
      });
    }
    expect(FakeAudio.made).toHaveLength(3);
    expect(result.current.now).toBeNull();
    expect(toast).toHaveBeenCalledTimes(1);
    expect(toast).toHaveBeenCalledWith("Could not play a: the file is not allowed or missing.", true);

    // A new run tells again.
    act(() => result.current.play(tracks[1]));
    await act(async () => {
      await Promise.resolve();
      vi.runAllTimers();
    });
    expect(toast).toHaveBeenCalledTimes(2);
    expect(result.current.now).toBeNull();
  });
});
