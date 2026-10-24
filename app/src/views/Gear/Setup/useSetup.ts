// The Setup segment's data: the radio's setup read against its card (else its latest backup),
// and `change`, which checks the next setup first. A setup that checks is saved (and the
// radio's open "Radio setup" change kept up to date); one with problems stays a draft on the
// page, with each problem at its job, until it checks.
import { useCallback, useEffect, useRef, useState } from "react";
import { api, errText } from "../../../ipc/api";
import type { RadioSetup, RadioSetupDetail, RadioSetupEditParams, StagedChange } from "../../../ipc/types";
import { useStore } from "../../../store";

/** The title of the change the setup stages (`core/radio_setup.rs`). */
export const SETUP_CHANGE = "Radio setup";

/** The radio's open "Radio setup" change. */
export const openSetupChange = (changes: StagedChange[], device: string | null) => changes.find((c) => c.device === device && c.title === SETUP_CHANGE && (c.status === "draft" || c.status === "ready"));

export interface UseSetup {
  detail: RadioSetupDetail | null;
  /** The setup on the page is not saved: it has problems. */
  draft: boolean;
  error: string | null;
  notes: string[];
  busy: boolean;
  /** Checks `next`, and saves it when it checks. */
  change: (next: RadioSetup) => Promise<void>;
  /** An edit of the saved setup (`fit`, `copy_from`, `reset`); answers the change it staged. */
  edit: (p: Omit<RadioSetupEditParams, "device">) => Promise<StagedChange | null>;
}

export function useSetup(device: string | null): UseSetup {
  const [detail, setDetail] = useState<RadioSetupDetail | null>(null);
  const [draft, setDraft] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notes, setNotes] = useState<string[]>([]);
  const [busy, setBusy] = useState(false);
  const loadGear = useStore((s) => s.loadGear);
  const staged = useStore((s) => !!openSetupChange(s.changes, device));
  const queue = useRef<Promise<unknown>>(Promise.resolve());

  useEffect(() => {
    if (!device) return;
    let gone = false;
    api.gearRadioSetup({ device }).then(
      (d) => {
        if (gone) return;
        setDetail(d);
        setDraft(false);
        setError(null);
      },
      (e) => {
        if (!gone) setError(errText(e));
      },
    );
    return () => {
      gone = true;
    };
  }, [device]);

  const run = useCallback(<T>(f: () => Promise<T>): Promise<T | null> => {
    const next = queue.current.then(async () => {
      setBusy(true);
      try {
        return await f();
      } catch (e) {
        setError(errText(e));
        return null;
      } finally {
        setBusy(false);
      }
    });
    queue.current = next;
    return next;
  }, []);

  const edit = useCallback(
    (p: Omit<RadioSetupEditParams, "device">) =>
      run(async () => {
        if (!device) return null;
        const r = await api.gearRadioSetupEdit({ device, ...p, stage: p.stage ?? staged });
        setDetail(r.detail);
        setDraft(false);
        setError(null);
        setNotes(r.notes);
        await loadGear();
        return r.change ?? null;
      }),
    [device, staged, run, loadGear],
  );

  const change = useCallback(
    async (next: RadioSetup) => {
      await run(async () => {
        if (!device) return;
        const d = await api.gearRadioSetup({ device, setup: next });
        if (d.problems?.length) {
          setDetail(d);
          setDraft(true);
          setError(null);
          return;
        }
        const r = await api.gearRadioSetupEdit({ device, setup: next, stage: staged });
        setDetail(r.detail);
        setDraft(false);
        setError(null);
        setNotes(r.notes);
        await loadGear();
      });
    },
    [device, staged, run, loadGear],
  );

  return { detail, draft, error, notes, busy, change, edit };
}
