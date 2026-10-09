// The model editors' data: one radio model read from the core with the device's staged model
// edits on top, and `stage`, which sends edits one after another and then lets the store
// refresh the staged changes (the read runs again when they change).
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { api, errText } from "../../../ipc/api";
import type { ModelDetail, ModelOp } from "../../../ipc/types";
import { useStore } from "../../../store";

export interface UseModel {
  detail: ModelDetail | null;
  /** The model file read now. */
  file: string | null;
  pick: (file: string) => void;
  error: string | null;
  /** A note that is not an error ("Nothing changes"). */
  note: string | null;
  /** Stages ops and, when given, the checklist text. Resolves when the core answered. */
  stage: (ops: ModelOp[], checklist?: string | null) => Promise<void>;
}

export function useModel(device: string | null): UseModel {
  const [file, setFile] = useState<string | null>(null);
  const [detail, setDetail] = useState<ModelDetail | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [editError, setEditError] = useState<string | null>(null);
  const [note, setNote] = useState<string | null>(null);
  const loadGear = useStore((s) => s.loadGear);
  const changes = useStore((s) => s.changes);
  const stagedKey = useMemo(() => JSON.stringify(changes.filter((c) => c.device === device).flatMap((c) => c.edits.filter((e) => e.kind === "model" || e.kind === "checklist"))), [changes, device]);
  const queue = useRef<Promise<void>>(Promise.resolve());

  const read = useCallback(async () => {
    if (!device) return;
    try {
      const d = await api.gearModel({ device, model: file, staged: true });
      setDetail(d);
      setLoadError(null);
    } catch (e) {
      setDetail(null);
      setLoadError(errText(e));
    }
  }, [device, file]);

  useEffect(() => {
    let gone = false;
    if (!device) return;
    api.gearModel({ device, model: file, staged: true }).then(
      (d) => {
        if (gone) return;
        setDetail(d);
        setLoadError(null);
      },
      (e) => {
        if (gone) return;
        setDetail(null);
        setLoadError(errText(e));
      },
    );
    return () => {
      gone = true;
    };
  }, [device, file, stagedKey]);

  const stage = useCallback(
    (ops: ModelOp[], checklist: string | null = null) => {
      const target = file ?? detail?.view.file;
      const run = queue.current.then(async () => {
        if (!device || !target) return;
        try {
          await api.gearModelEdit({ device, model: target, ops, checklist });
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
        await read();
      });
      queue.current = run;
      return run;
    },
    [device, file, detail, loadGear, read],
  );

  return { detail, file: file ?? detail?.view.file ?? null, pick: setFile, error: loadError ?? editError, note, stage };
}
