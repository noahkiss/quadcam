// The Settings editor's data: the radio's settings read from the core twice, as the radio has
// them and with the device's staged radio edits on top, and `stage`, which sends edits one
// after another and then lets the store refresh the staged changes (the reads run again when
// they change).
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { api, errText } from "../../../ipc/api";
import type { RadioOp, RadioSettingsDetail } from "../../../ipc/types";
import { useStore } from "../../../store";

export interface UseRadioSettings {
  /** With the staged edits on top. */
  detail: RadioSettingsDetail | null;
  /** As the radio has them. */
  base: RadioSettingsDetail | null;
  error: string | null;
  /** A note that is not an error ("Nothing changes"). */
  note: string | null;
  stage: (ops: RadioOp[]) => Promise<void>;
}

export function useRadioSettings(device: string | null): UseRadioSettings {
  const [detail, setDetail] = useState<RadioSettingsDetail | null>(null);
  const [base, setBase] = useState<RadioSettingsDetail | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [editError, setEditError] = useState<string | null>(null);
  const [note, setNote] = useState<string | null>(null);
  const loadGear = useStore((s) => s.loadGear);
  const changes = useStore((s) => s.changes);
  const stagedKey = useMemo(() => JSON.stringify(changes.filter((c) => c.device === device).flatMap((c) => [c.status, ...c.edits.filter((e) => e.kind === "radio")])), [changes, device]);
  const queue = useRef<Promise<void>>(Promise.resolve());
  const [tick, setTick] = useState(0);

  useEffect(() => {
    let gone = false;
    if (!device) return;
    Promise.all([api.gearRadioSettings({ device, staged: true }), api.gearRadioSettings({ device, staged: false })]).then(
      ([d, b]) => {
        if (gone) return;
        setDetail(d);
        setBase(b);
        setLoadError(null);
      },
      (e) => {
        if (gone) return;
        setDetail(null);
        setBase(null);
        setLoadError(errText(e));
      },
    );
    return () => {
      gone = true;
    };
  }, [device, stagedKey, tick]);

  const stage = useCallback(
    (ops: RadioOp[]) => {
      const run = queue.current.then(async () => {
        if (!device) return;
        try {
          await api.gearRadioSettingsEdit({ device, ops });
          setEditError(null);
          setNote(null);
        } catch (e) {
          const t = errText(e);
          if (t.startsWith("Nothing changes")) {
            setNote(t);
            setEditError(null);
          } else setEditError(t);
        }
        await loadGear();
        setTick((n) => n + 1);
      });
      queue.current = run;
      return run;
    },
    [device, loadGear],
  );

  return { detail, base, error: loadError ?? editError, note, stage };
}
