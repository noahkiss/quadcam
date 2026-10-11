// The Voice studio's data (ElevenLabs): the key's state, the account's voices, models and
// credits, the line sets, the estimate for the picked voice, model and sets, and the last
// sample. A paid call runs in two steps: the first asks the core, which answers with the
// cost, `needs_confirm` and the plan's digest; the second repeats it with `confirm` and that
// digest, and the core refuses it when the plan changed. A picker change drops the question.
import { useCallback, useEffect, useState } from "react";
import { api, errText } from "../../../ipc/api";
import type { Catalog, KeyStatus, RenderReport, SampleReport, SetInfo, StudioEstimate } from "../../../ipc/types";

export type Paid = "sample" | "render";

export interface UseStudio {
  key: KeyStatus | null;
  sets: SetInfo[];
  catalog: Catalog | null;
  /** Ids of the voices, the models and the sets the person ticked. */
  voices: string[];
  models: string[];
  picked: string[];
  setVoices: (v: string[]) => void;
  setModels: (v: string[]) => void;
  setPicked: (v: string[]) => void;
  estimate: StudioEstimate | null;
  sample: SampleReport | null;
  render: RenderReport | null;
  /** A paid call waits for the person's go-ahead. */
  asking: Paid | null;
  busy: boolean;
  error: string | null;
  saveKey: (key: string) => Promise<void>;
  deleteKey: () => Promise<void>;
  /** Starts a paid call; `go` is the go-ahead. */
  run: (what: Paid, go: boolean) => Promise<void>;
  /** Renders the pack again with a new seed: only the batches with a line that needs a re-take. */
  retake: () => Promise<void>;
  cancel: () => void;
}

export function useStudio(onRendered: () => Promise<void>): UseStudio {
  const [key, setKey] = useState<KeyStatus | null>(null);
  const [sets, setSets] = useState<SetInfo[]>([]);
  const [catalog, setCatalog] = useState<Catalog | null>(null);
  const [voices, pickVoices] = useState<string[]>([]);
  const [models, pickModels] = useState<string[]>([]);
  const [picked, pickSets] = useState<string[]>(["quad"]);
  const [priced, setPriced] = useState<{ k: string; e: StudioEstimate } | null>(null);
  /** Counts finished paid calls: the cache changed, so the estimate is read again. */
  const [ran, setRan] = useState(0);
  const [sample, setSample] = useState<SampleReport | null>(null);
  const [render, setRender] = useState<RenderReport | null>(null);
  const [asking, setAsking] = useState<Paid | null>(null);
  /** The asked call's digest and seed: the go-ahead repeats both. */
  const [plan, setPlan] = useState<{ digest: string; seed: number } | null>(null);
  const stop = useCallback(() => {
    setAsking(null);
    setPlan(null);
  }, []);
  const setVoices = useCallback((v: string[]) => (pickVoices(v), stop()), [stop]);
  const setModels = useCallback((v: string[]) => (pickModels(v), stop()), [stop]);
  const setPicked = useCallback((v: string[]) => (pickSets(v), stop()), [stop]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const loadCatalog = useCallback(async () => {
    try {
      const c = await api.gearVoiceCatalog({ voices: false, models: false, credits: false });
      setCatalog(c);
      pickVoices((v) => (v.length ? v : c.voices.slice(0, 1).map((x) => x.id)));
      pickModels((m) => (m.length ? m : c.models.slice(0, 1).map((x) => x.id)));
    } catch (e) {
      setCatalog(null);
      setError(errText(e));
    }
  }, []);

  useEffect(() => {
    let gone = false;
    api.gearVoiceSets().then(
      (v) => {
        if (gone) return;
        setKey(v.key);
        setSets(v.sets);
        if (v.key.set) void loadCatalog();
      },
      (e) => {
        if (!gone) setError(errText(e));
      },
    );
    return () => {
      gone = true;
    };
  }, [loadCatalog]);

  const keySet = key?.set ?? false;
  const v0 = voices[0];
  const m0 = models[0];
  const want = [keySet, v0, m0, picked.join(","), ran].join("|");
  useEffect(() => {
    let gone = false;
    if (!keySet || !v0 || !m0 || picked.length === 0) return;
    api.gearVoiceEstimate({ sets: picked, voice: v0, model: m0, lines: [], settings: null, batch: null }).then(
      (e) => {
        if (!gone) setPriced({ k: want, e });
      },
      (e) => {
        if (!gone) setError(errText(e));
      },
    );
    return () => {
      gone = true;
    };
  }, [keySet, v0, m0, picked, want]);
  const estimate = priced && priced.k === want ? priced.e : null;

  const saveKey = useCallback(
    async (k: string) => {
      try {
        const s = await api.gearVoiceKey({ action: "set", key: k });
        setKey(s);
        setError(null);
        await loadCatalog();
      } catch (e) {
        setError(errText(e));
      }
    },
    [loadCatalog],
  );

  const deleteKey = useCallback(async () => {
    try {
      setKey(await api.gearVoiceKey({ action: "delete", key: null }));
      setCatalog(null);
      setSample(null);
      setError(null);
    } catch (e) {
      setError(errText(e));
    }
  }, []);

  const call = useCallback(
    async (what: Paid, go: boolean, newSeed?: number) => {
      setBusy(true);
      setError(null);
      const seed = go ? (plan?.seed ?? 0) : (newSeed ?? 0);
      const digest = go ? (plan?.digest ?? null) : null;
      const settings = seed ? { seed } : null;
      try {
        if (what === "sample") {
          const r = await api.gearVoiceSample({ voices, models, sets: ["sample"], lines: [], dry_run: false, confirm: go, settings, batch: null, digest });
          setSample(r);
          setAsking(r.needs_confirm ? "sample" : null);
          setPlan(r.needs_confirm ? { digest: r.digest ?? "", seed } : null);
        } else {
          const r = await api.gearVoiceRender({ voice: v0 ?? "", lines: [], dry_run: false, confirm: go, settings, sets: picked, model: m0 ?? "", batch: null, digest });
          setRender(r);
          setAsking(r.needs_confirm ? "render" : null);
          setPlan(r.needs_confirm ? { digest: r.digest ?? "", seed } : null);
          if (!r.needs_confirm) await onRendered();
        }
        // The credits changed.
        void api.gearVoiceCatalog({ voices: false, models: false, credits: true }).then((c) => setCatalog((o) => (o ? { ...o, credits: c.credits } : o)));
      } catch (e) {
        setError(errText(e));
        stop();
      } finally {
        setRan((r) => r + 1);
        setBusy(false);
      }
    },
    [voices, models, picked, v0, m0, onRendered, plan, stop],
  );
  const run = useCallback((what: Paid, go: boolean) => call(what, go), [call]);
  // Any seed but 0 makes new takes; the pack keeps every batch whose lines all passed.
  const retake = useCallback(() => call("render", false, 1 + Math.floor(Math.random() * 2_000_000_000)), [call]);

  return { key, sets, catalog, voices, models, picked, setVoices, setModels, setPicked, estimate, sample, render, asking, busy, error, saveKey, deleteKey, run, retake, cancel: stop };
}
