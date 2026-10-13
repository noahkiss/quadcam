// The Voice studio's data (ElevenLabs): the key's state, the account's voices, models and
// credits, the line sets, the estimate for every picked voice and model with the sets, the
// last sample, and the progress of a paid run. A paid call runs in two steps: the first asks the core, which answers with the
// cost, `needs_confirm` and the plan's digest; the second repeats it with `confirm` and that
// digest, and the core refuses it when the plan changed. A picker change drops the question.
import { useCallback, useEffect, useState } from "react";
import { api, errText } from "../../../ipc/api";
import { on } from "../../../ipc/events";
import type { Catalog, ImportReport, KeyStatus, RenderPacksReport, SampleReport, SetInfo, TakeCount, VoiceProgress } from "../../../ipc/types";

/** The gear job a paid studio run registers; `gear_stop` with it stops after the batch. */
export const STUDIO_JOB = "voice-studio";

export type Paid = "sample" | "render";

/** Lines a voice (and model) have a take of: the best of their models and speeds. */
export function takesOf(takes: TakeCount[], voice: string, model?: string) {
  return takes.filter((t) => t.voice === voice && (!model || t.model === model)).reduce((m, t) => Math.max(m, t.lines), 0);
}

export interface UseStudio {
  key: KeyStatus | null;
  /** The sets; with one voice and one model ticked, each says how many lines are rendered. */
  sets: SetInfo[];
  /** The provider's speaking speed: 1, or another speed earlier takes were made at. */
  speed: number;
  setSpeed: (v: number) => void;
  /** Copies a folder of earlier takes into the cache. */
  importTakes: (folder: string) => Promise<void>;
  imported: ImportReport | null;
  /** The aircraft kinds the person flies (from the profiles). */
  aircraft: string[];
  /** The lines each voice, model and speed have a take of. */
  takes: TakeCount[];
  catalog: Catalog | null;
  /** Ids of the voices, the models and the sets the person ticked. */
  voices: string[];
  models: string[];
  picked: string[];
  setVoices: (v: string[]) => void;
  setModels: (v: string[]) => void;
  setPicked: (v: string[]) => void;
  /** Every ticked voice with every ticked model: each pair and the total. */
  estimate: RenderPacksReport | null;
  sample: SampleReport | null;
  render: RenderPacksReport | null;
  /** The paid run going on now, from the core's `voice-progress` events. */
  progress: VoiceProgress | null;
  /** Stops the paid run after the current batch. */
  stopRun: () => Promise<void>;
  /** Reads the account's voices, models and credits again (after a voice is added). */
  refreshCatalog: () => Promise<void>;
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

/** The height of the studio's voice lists: they scroll inside it, under their header. */
export const LIST_HEIGHT = "min(60vh, 480px)";

export function useStudio(onRendered: () => Promise<void>, onSampled: () => Promise<void> = async () => undefined): UseStudio {
  const [key, setKey] = useState<KeyStatus | null>(null);
  const [sets, setSets] = useState<SetInfo[]>([]);
  const [aircraft, setAircraft] = useState<string[]>([]);
  const [catalog, setCatalog] = useState<Catalog | null>(null);
  const [voices, pickVoices] = useState<string[]>([]);
  const [models, pickModels] = useState<string[]>([]);
  const [picked, pickSets] = useState<string[]>([]);
  const [priced, setPriced] = useState<{ k: string; e: RenderPacksReport } | null>(null);
  /** Counts finished paid calls: the cache changed, so the estimate is read again. */
  const [ran, setRan] = useState(0);
  const [sample, setSample] = useState<SampleReport | null>(null);
  const [render, setRender] = useState<RenderPacksReport | null>(null);
  const [running, setRunning] = useState<VoiceProgress | null>(null);
  useEffect(() => {
    let off: (() => void) | undefined;
    let gone = false;
    void on("voice-progress", (p) => setRunning(p.running ? p : null)).then((u) => {
      if (gone) u();
      else off = u;
    });
    return () => {
      gone = true;
      off?.();
    };
  }, []);
  const [asking, setAsking] = useState<Paid | null>(null);
  /** The asked call's digest and seed: the go-ahead repeats both. */
  const [plan, setPlan] = useState<{ digest: string; seed: number } | null>(null);
  const stop = useCallback(() => {
    setAsking(null);
    setPlan(null);
  }, []);
  const setVoices = useCallback((v: string[]) => (pickVoices(v), stop()), [stop]);
  // The window renders with one model at a time; the core still takes several.
  const setModels = useCallback((v: string[]) => (pickModels(v.slice(0, 1)), stop()), [stop]);
  /** A tick saves the sets in the gear folder, for the next time the studio opens. */
  const setPicked = useCallback(
    (v: string[]) => {
      pickSets(v);
      stop();
      api.gearVoiceSaveSets({ sets: v }).catch((e) => setError(errText(e)));
    },
    [stop],
  );
  const [speed, pickSpeed] = useState(1);
  const setSpeed = useCallback((v: number) => (pickSpeed(v), stop()), [stop]);
  const [imported, setImported] = useState<ImportReport | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const loadCatalog = useCallback(async () => {
    try {
      const c = await api.gearVoiceCatalog({ voices: false, models: false, credits: false });
      setCatalog(c);
      pickVoices((v) => (v.length ? v : c.voices.slice(0, 1).map((x) => x.id)));
      pickModels((m) => (m.length && c.models.some((x) => x.id === m[0]) ? m.slice(0, 1) : c.models.slice(0, 1).map((x) => x.id)));
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
        setAircraft(v.aircraft ?? []);
        pickSets((p) => (p.length ? p : (v.picks ?? [])));
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
  const want = [keySet, voices.join(","), models.join(","), picked.join(","), speed, ran].join("|");
  useEffect(() => {
    let gone = false;
    if (!keySet || voices.length === 0 || models.length === 0 || picked.length === 0) return;
    api.gearVoiceRenderPacks({ voices, models, sets: picked, lines: [], dry_run: true, confirm: false, settings: speed === 1 ? null : { speed }, batch: null, digest: null }).then(
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
  }, [keySet, voices, models, picked, speed, want]);
  const estimate = priced && priced.k === want ? priced.e : null;

  // The takes per voice and, with one voice and one model ticked, how many lines of each set
  // they have rendered (no network). Read again after every paid call.
  const single = voices.length === 1 && models.length === 1 && !!v0 && !!m0;
  const progressKey = [single ? v0 : "", single ? m0 : "", speed, ran].join("|");
  const [progress, setProgress] = useState<{ k: string; sets: SetInfo[]; takes: TakeCount[] } | null>(null);
  useEffect(() => {
    let gone = false;
    const asked = single ? { voice: v0 ?? "", model: m0 ?? "", settings: speed === 1 ? null : { speed }, batch: null } : undefined;
    api.gearVoiceSets(asked).then(
      (v) => {
        if (!gone) setProgress({ k: progressKey, sets: v.sets, takes: v.rendered ?? [] });
      },
      () => undefined,
    );
    return () => {
      gone = true;
    };
  }, [progressKey, single, v0, m0, speed]);
  const fresh = progress && progress.k === progressKey ? progress : null;
  const shownSets = fresh && single ? fresh.sets : sets.map((x) => ({ ...x, rendered: null }));
  const takes = progress?.takes ?? [];

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
      const settings = seed || speed !== 1 ? { seed, speed } : null;
      try {
        if (what === "sample") {
          const r = await api.gearVoiceSample({ voices, models, sets: ["sample"], lines: [], dry_run: false, confirm: go, settings, batch: null, digest });
          setSample(r);
          setAsking(r.needs_confirm ? "sample" : null);
          setPlan(r.needs_confirm ? { digest: r.digest ?? "", seed } : null);
          if (!r.needs_confirm) await onSampled();
        } else {
          const r = await api.gearVoiceRenderPacks({ voices, models, sets: picked, lines: [], dry_run: false, confirm: go, settings, batch: null, digest });
          setRender(r);
          setAsking(r.needs_confirm ? "render" : null);
          setPlan(r.needs_confirm ? { digest: r.digest, seed } : null);
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
        setRunning(null);
      }
    },
    [voices, models, picked, speed, onRendered, onSampled, plan, stop],
  );
  const importTakes = useCallback(async (folder: string) => {
    setError(null);
    try {
      setImported(await api.gearVoiceImportTakes({ folder, dry_run: false }));
    } catch (e) {
      setError(errText(e));
    } finally {
      setRan((r) => r + 1);
    }
  }, []);
  const run = useCallback((what: Paid, go: boolean) => call(what, go), [call]);
  const stopRun = useCallback(async () => {
    try {
      await api.gearStop(STUDIO_JOB);
    } catch (e) {
      setError(errText(e));
    }
  }, []);
  // Any seed but 0 makes new takes; the pack keeps every batch whose lines all passed.
  const retake = useCallback(() => call("render", false, 1 + Math.floor(Math.random() * 2_000_000_000)), [call]);

  return { key, sets: shownSets, aircraft, takes, speed, setSpeed, importTakes, imported, catalog, voices, models, picked, setVoices, setModels, setPicked, estimate, sample, render, progress: running, stopRun, refreshCatalog: loadCatalog, asking, busy, error, saveKey, deleteKey, run, retake, cancel: stop };
}
