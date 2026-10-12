// One player for the sample gallery: a take, or a queue of takes played in order (Play all),
// with the take playing now and Stop. Starting a new play stops the one before it.
import { useCallback, useEffect, useRef, useState } from "react";
import { fileSrc } from "../../../ipc/api";
import { toast } from "../../../components/toastStore";

export interface Track {
  /** The card the take is on. */
  card: string;
  line: string;
  file: string;
}

export interface Player {
  /** The take playing now. */
  now: Track | null;
  /** The card whose takes are queued, or "all" for every card; null for one take. */
  queue: string | null;
  play: (t: Track) => void;
  playAll: (tracks: Track[], queue: string) => void;
  stop: () => void;
}

/** The gap between two takes of a queue, so the lines do not run together. */
const GAP_MS = 350;

/** The toast for a take the web view could not load or play. */
export const playError = (line: string) => `Could not play ${line}: the file is not allowed or missing.`;

export function usePlayer(): Player {
  const [now, setNow] = useState<Track | null>(null);
  const [queue, setQueue] = useState<string | null>(null);
  const audio = useRef<HTMLAudioElement | null>(null);
  const timer = useRef<number | null>(null);
  const run = useRef(0);

  const halt = useCallback(() => {
    run.current++;
    if (timer.current !== null) window.clearTimeout(timer.current);
    timer.current = null;
    if (audio.current) {
      audio.current.onended = null;
      audio.current.onerror = null;
      audio.current.pause();
    }
    audio.current = null;
  }, []);

  const stop = useCallback(() => {
    halt();
    setNow(null);
    setQueue(null);
  }, [halt]);

  const start = useCallback(
    (tracks: Track[], q: string | null) => {
      halt();
      const id = run.current;
      // One toast per run, however many takes of it fail.
      let told = false;
      setQueue(q);
      const next = (i: number) => {
        if (id !== run.current) return;
        const t = tracks[i];
        if (!t) {
          setNow(null);
          setQueue(null);
          return;
        }
        setNow(t);
        const a = new Audio(fileSrc(t.file));
        audio.current = a;
        let done = false;
        const after = (failed: boolean) => {
          a.onended = null;
          a.onerror = null;
          // onerror and the play() rejection both report one failure.
          if (done || id !== run.current) return;
          done = true;
          if (failed && !told) {
            told = true;
            toast(playError(t.line), true);
          }
          timer.current = window.setTimeout(() => next(i + 1), tracks.length > 1 ? GAP_MS : 0);
        };
        a.onended = () => after(false);
        a.onerror = () => after(true);
        void a.play().catch(() => after(true));
      };
      next(0);
    },
    [halt],
  );

  useEffect(() => halt, [halt]);

  const play = useCallback((t: Track) => start([t], null), [start]);
  const playAll = useCallback((tracks: Track[], q: string) => start(tracks, q), [start]);
  return { now, queue, play, playAll, stop };
}
