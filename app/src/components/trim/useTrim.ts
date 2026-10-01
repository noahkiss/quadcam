import { useCallback, useEffect, useRef, useState, type RefObject } from "react";
import type { Moment, Span } from "../../ipc/types";
import { toast } from "../toastStore";
import { FRAME, frameMoment, withCut, withKeep, without, type Sel, type TrimModel } from "./model";

export interface TrimOptions {
  /** Sends a new cut list to the core. */
  setCuts: (cuts: Span[]) => Promise<void>;
  /** Moves the radio log against the clip. */
  setOffset?: (s: number) => void;
  /** Writes the unsaved cuts (the library only). */
  save?: () => Promise<void>;
}

export interface Trim {
  model: TrimModel | null;
  sel: Sel;
  setSel: (s: Sel) => void;
  /** The video's current time, or null before it plays. */
  head: number | null;
  video: RefObject<HTMLVideoElement | null>;
  opts: TrimOptions;
  seek: (t: number) => void;
  frame: (m: Moment) => void;
  setPoint: (which: "in" | "out") => void;
  /** Adds the in-out range as a cut; `over` uses other points (a moment's). */
  addCut: (over?: Sel) => Promise<void>;
  removeCut: (i: number) => Promise<void>;
  useKeep: () => Promise<void>;
  armHere: () => void;
  /** The editor's keys. True when it handled the key. */
  handleKey: (e: KeyboardEvent) => boolean;
}

/** The trim editor's state: in and out points, the playhead, and the actions on them. */
export function useTrim(model: TrimModel | null, video: RefObject<HTMLVideoElement | null>, opts: TrimOptions): Trim {
  const [sel, setSel] = useState<Sel>({ in: null, out: null });
  const [head, setHead] = useState<number | null>(null);
  const [key, setKey] = useState(model?.key);
  if (model?.key !== key) {
    setKey(model?.key);
    setSel({ in: null, out: null });
  }

  useEffect(() => {
    const v = video.current;
    if (!v) return;
    const on = () => setHead(v.src && !v.hidden ? v.currentTime : null);
    v.addEventListener("timeupdate", on);
    v.addEventListener("seeked", on);
    v.addEventListener("emptied", on);
    return () => {
      v.removeEventListener("timeupdate", on);
      v.removeEventListener("seeked", on);
      v.removeEventListener("emptied", on);
    };
  }, [video, model?.key]);

  const playing = () => {
    const v = video.current;
    return v && v.getAttribute("src") ? v : null;
  };
  const seek = useCallback(
    (t: number) => {
      const v = video.current;
      if (v && v.getAttribute("src")) v.currentTime = Math.max(0, Math.min(t, model?.duration || t));
    },
    [video, model?.duration],
  );

  // The latest values for the key handler, which outlives renders.
  const latest = useRef({ model, sel, head, opts });
  useEffect(() => {
    latest.current = { model, sel, head, opts };
  });

  const t: Trim = {
    model,
    sel,
    setSel,
    head,
    video,
    opts,
    seek,
    frame: (m) => {
      if (!model) return;
      const s = frameMoment(m, model.duration);
      setSel(s);
      seek(s.in!);
    },
    setPoint: (which) => {
      const v = playing();
      if (!v) return toast("Play the clip first, or type the time.", true);
      setSel((s) => ({ ...s, [which]: +v.currentTime.toFixed(1) }));
    },
    addCut: async (over) => {
      const { model: m, sel: s, opts: o } = latest.current;
      if (!m) return;
      const cuts = withCut(m.cuts, over ?? s);
      if (typeof cuts === "string") return toast(cuts, true);
      setSel({ in: null, out: null });
      await o.setCuts(cuts);
    },
    removeCut: async (i) => {
      if (model) await opts.setCuts(without(model.cuts, i));
    },
    useKeep: async () => {
      if (model) await opts.setCuts(withKeep(model.cuts, model.keep));
    },
    armHere: () => {
      const v = playing();
      if (!v) return toast("Play the clip to where you armed first.", true);
      opts.setOffset?.(+v.currentTime.toFixed(1));
    },
    handleKey: () => false,
  };
  t.handleKey = (e) => {
    if (!latest.current.model) return false;
    // Command, Control and Option shortcuts belong to the app (Command-I is Edit details).
    if (e.metaKey || e.ctrlKey || e.altKey) return false;
    const v = playing();
    const k = e.key.length === 1 ? e.key.toLowerCase() : e.key;
    switch (k) {
      case " ":
        if (!v) return false;
        if (v.paused) v.play();
        else v.pause();
        return true;
      case "j":
        if (v) v.currentTime = Math.max(0, v.currentTime - 5);
        return true;
      case "k":
        if (video.current) {
          video.current.pause();
          video.current.playbackRate = 1;
        }
        return true;
      case "l":
        if (v) {
          if (v.paused) v.play();
          else v.playbackRate = Math.min(4, v.playbackRate * 2);
        }
        return true;
      case "ArrowLeft":
      case "ArrowRight":
        if (!v) return false;
        v.pause();
        v.currentTime = Math.max(0, v.currentTime + (k === "ArrowLeft" ? -FRAME : FRAME));
        return true;
      case "i":
        t.setPoint("in");
        return true;
      case "o":
        t.setPoint("out");
        return true;
      case "c":
        t.addCut();
        return true;
      default:
        return false;
    }
  };
  return t;
}
