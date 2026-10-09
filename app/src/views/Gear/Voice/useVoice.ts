// The Voice segment's data: the lines, the packs and the provider for one radio, read again
// after each change and whenever the radio's staged changes move.
import { useCallback, useEffect, useMemo, useState } from "react";
import { api, errText } from "../../../ipc/api";
import type { VoiceView } from "../../../ipc/types";
import { useStore } from "../../../store";

export interface UseVoice {
  view: VoiceView | null;
  error: string | null;
  setError: (e: string | null) => void;
  /** Runs a call, shows its error, then reads the view again. */
  act: <T>(f: () => Promise<T>) => Promise<T | undefined>;
  refresh: () => Promise<void>;
}

export function useVoice(radio: string | null): UseVoice {
  const [view, setView] = useState<VoiceView | null>(null);
  const [error, setError] = useState<string | null>(null);
  const loadGear = useStore((s) => s.loadGear);
  const changes = useStore((s) => s.changes);
  const key = useMemo(() => JSON.stringify(changes.filter((c) => c.device === radio).map((c) => [c.id, c.status, c.title])), [changes, radio]);

  const read = useCallback(
    async (refreshIndex = false) => {
      if (!radio) return;
      try {
        setView(await api.gearVoice({ radio, refresh_index: refreshIndex }));
      } catch (e) {
        setError(errText(e));
      }
    },
    [radio],
  );

  useEffect(() => {
    let gone = false;
    if (!radio) return;
    api.gearVoice({ radio, refresh_index: false }).then(
      (v) => {
        if (!gone) setView(v);
      },
      (e) => {
        if (!gone) setError(errText(e));
      },
    );
    return () => {
      gone = true;
    };
  }, [radio, key]);

  const act = useCallback(
    async <T,>(f: () => Promise<T>) => {
      try {
        const r = await f();
        setError(null);
        await loadGear();
        await read();
        return r;
      } catch (e) {
        setError(errText(e));
        return undefined;
      }
    },
    [loadGear, read],
  );

  return { view, error, setError, act, refresh: () => read(true) };
}
