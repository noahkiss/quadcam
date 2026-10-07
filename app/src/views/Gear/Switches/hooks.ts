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

/** Latest backups to read when no file is picked: an aircraft's radio and FC, or devices. */
export interface BackupSource {
  aircraft?: string | null;
  devices?: string[];
}

/** The params for the map: the files picked, else the backups. */
export const mapParams = (src: MapSources, backup: BackupSource | null) => (src.radio || src.fc.length ? { radio: src.radio, fc: src.fc } : { aircraft: backup?.aircraft ?? null, devices: backup?.devices ?? [] });

/** The switch map from the files picked (else the latest backups), and the pickers. */
export function useSwitchMap(backup: BackupSource | null = null) {
  const [src, setSrcState] = useState<MapSources>(remembered);
  const [map, setMap] = useState<SwitchMap | null>(null);
  const [error, setError] = useState<string | null>(null);
  const setSrc = (s: MapSources) => {
    remembered = s;
    setSrcState(s);
  };

  const key = JSON.stringify(backup);
  useEffect(() => {
    if (!src.radio && !src.fc.length && !backup) return;
    let gone = false;
    api.gearSwitchMap(mapParams(src, backup)).then(
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
    // eslint-disable-next-line react-hooks/exhaustive-deps -- `key` stands for `backup`
  }, [src, key]);

  return {
    src,
    params: mapParams(src, backup),
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
