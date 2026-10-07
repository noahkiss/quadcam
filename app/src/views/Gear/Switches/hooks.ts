// The switch map's sources and the radio stream, shared by the Switches segment and the
// Controls page. The files picked last stay picked while the app runs.
import { useEffect, useState } from "react";
import { api, errText, pickFiles, pickFolder } from "../../../ipc/api";
import { on } from "../../../ipc/events";
import type { RadioEvent, SwitchMap } from "../../../ipc/types";

export interface MapSources {
  /** An EdgeTX card folder or one model file. */
  radio: string | null;
  /** Betaflight dump, diff or CLI files. */
  fc: string[];
}

let remembered: MapSources = { radio: null, fc: [] };

/** The switch map from the files picked, and the pickers. */
export function useSwitchMap() {
  const [src, setSrcState] = useState<MapSources>(remembered);
  const [map, setMap] = useState<SwitchMap | null>(null);
  const [error, setError] = useState<string | null>(null);
  const setSrc = (s: MapSources) => {
    remembered = s;
    setSrcState(s);
  };

  useEffect(() => {
    if (!src.radio && !src.fc.length) return;
    let gone = false;
    api.gearSwitchMap({ radio: src.radio, fc: src.fc }).then(
      (m) => {
        if (gone) return;
        setMap(m);
        setError(null);
      },
      (e) => {
        if (!gone) setError(errText(e));
      },
    );
    return () => {
      gone = true;
    };
  }, [src]);

  return {
    src,
    map,
    error,
    openCard: async () => {
      const f = await pickFolder("Open an EdgeTX card or a copy of one");
      if (f) setSrc({ ...src, radio: f });
    },
    openModel: async () => {
      const [f] = await pickFiles("Open an EdgeTX model file", [{ name: "EdgeTX model", extensions: ["yml"] }]);
      if (f) setSrc({ ...src, radio: f });
    },
    openDump: async () => {
      const picked = await pickFiles("Open a Betaflight dump or diff", [{ name: "Betaflight CLI text", extensions: ["txt", "cli"] }]);
      if (picked.length) setSrc({ ...src, fc: picked });
    },
  };
}

/** The radio in USB Joystick mode while `on`: its latest event, at most one per frame. */
export function useRadio(enabled = true): RadioEvent | null {
  const [ev, setEv] = useState<RadioEvent | null>(null);
  useEffect(() => {
    if (!enabled) return;
    let gone = false;
    let raf = 0;
    let latest: RadioEvent | null = null;
    const unlisten = on("radio-input", (e) => {
      latest = e;
      if (!raf)
        raf = requestAnimationFrame(() => {
          raf = 0;
          if (!gone) setEv(latest);
        });
    });
    api.gearRadioWatch(true).catch((e) => console.warn("radio stream", errText(e)));
    return () => {
      gone = true;
      cancelAnimationFrame(raf);
      void unlisten.then((f) => f());
      api.gearRadioWatch(false).catch(() => {});
    };
  }, [enabled]);
  return enabled ? ev : null;
}
