// The sample gallery's data: every sample voice and pack on this Mac as the core lists them
// (`gear_voice_samples`), read when the page opens and again after each change. Flags and
// deletes change only QuadCam's files; a re-take is a paid call in two steps, like the
// studio's: the first answers the cost and a digest, the second repeats the digest.
import { useCallback, useEffect, useState } from "react";
import { api, errText } from "../../../ipc/api";
import type { RetakeReport, SampleGallery, SampleVoice } from "../../../ipc/types";

export interface FlagInput {
  reason: string;
  note: string;
}

export interface UseGallery {
  gallery: SampleGallery | null;
  error: string | null;
  busy: boolean;
  reload: () => Promise<void>;
  /** Deletes sample voices by id, or every sample. */
  remove: (ids: string[] | "all") => Promise<void>;
  /** Flags a take, or clears its flag with `null`. */
  flag: (card: SampleVoice, line: string, f: FlagInput | null) => Promise<void>;
  /** A re-take of a card's flagged takes: priced first (`go` false), then paid with the digest. */
  retake: (card: SampleVoice, go: string | null) => Promise<RetakeReport | null>;
}

export function useGallery(): UseGallery {
  const [gallery, setGallery] = useState<SampleGallery | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const reload = useCallback(async () => {
    try {
      setGallery(await api.gearVoiceSamples());
    } catch (e) {
      setError(errText(e));
    }
  }, []);

  useEffect(() => {
    let gone = false;
    api.gearVoiceSamples().then(
      (g) => {
        if (!gone) setGallery(g);
      },
      (e) => {
        if (!gone) setError(errText(e));
      },
    );
    return () => {
      gone = true;
    };
  }, []);

  const act = useCallback(async <T>(f: () => Promise<T>): Promise<T | null> => {
    setBusy(true);
    setError(null);
    try {
      return await f();
    } catch (e) {
      setError(errText(e));
      return null;
    } finally {
      setBusy(false);
    }
  }, []);

  const remove = useCallback(
    async (ids: string[] | "all") => {
      const g = await act(() => api.gearVoiceSamplesDelete(ids === "all" ? { ids: [], all: true } : { ids, all: false }));
      if (g) setGallery(g);
    },
    [act],
  );

  const flag = useCallback(
    async (card: SampleVoice, line: string, f: FlagInput | null) => {
      const target = card.kind === "pack" ? { sample: "", pack: card.id } : { sample: card.id, pack: "" };
      const done = await act(() => api.gearVoiceFlag({ ...target, line, reason: f?.reason ?? "", note: f?.note ?? "", clear: !f }));
      if (done) await reload();
    },
    [act, reload],
  );

  const retake = useCallback(
    async (card: SampleVoice, go: string | null) => {
      const target = card.kind === "pack" ? { sample: "", pack: card.id } : { sample: card.id, pack: "" };
      const r = await act(() => api.gearVoiceRetake({ ...target, lines: [], dry_run: false, confirm: go !== null, digest: go }));
      if (r && !r.needs_confirm) await reload();
      return r;
    },
    [act, reload],
  );

  return { gallery, error, busy, reload, remove, flag, retake };
}
