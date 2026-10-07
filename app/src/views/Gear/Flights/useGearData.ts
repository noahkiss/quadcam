// Loads a Gear answer and loads it again when Gear data changes (`gear-changed` reloads the
// store's status), when the library changes, or when `deps` change.
import { useCallback, useEffect, useState } from "react";
import { useStore } from "../../../store";
import { errText } from "../../../ipc/api";

export interface GearData<T> {
  data: T | null;
  error: string | null;
  reload: () => void;
}

export function useGearData<T>(load: () => Promise<T>, deps: unknown[] = []): GearData<T> {
  const gear = useStore((s) => s.gear);
  const lib = useStore((s) => s.lib);
  const [data, setData] = useState<T | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [tick, setTick] = useState(0);
  const reload = useCallback(() => setTick((t) => t + 1), []);
  useEffect(() => {
    let live = true;
    load()
      .then((d) => {
        if (!live) return;
        setData(d);
        setError(null);
      })
      .catch((e) => live && setError(errText(e)));
    return () => {
      live = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [gear, lib, tick, ...deps]);
  return { data, error, reload };
}
