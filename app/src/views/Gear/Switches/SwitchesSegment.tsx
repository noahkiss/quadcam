// The Switches segment (design 7.2): what each radio control does, from an EdgeTX card or
// model file and the FC's dump, with the position each control is in now. Live comes from
// the radio in USB Joystick mode, or from the FC's channels over MSP once a second. The FC
// and radio device pages mount it (`views/Gear/segments.tsx`); WP4's backups replace the
// file pickers as the default source.
import { useEffect, useState } from "react";
import { Banner } from "../../../components/Banner";
import { SegmentedControl } from "../../../components/SegmentedControl";
import { api, errText } from "../../../ipc/api";
import type { Live } from "../../../ipc/types";
import { live as liveOf } from "../../../lib/controls";
import { useStore } from "../../../store";
import type { DeviceRef } from "../slots";
import { useRadio, useSwitchMap } from "./hooks";
import { MapSourceBar, SwitchMapView } from "./SwitchMapView";
import styles from "./Switches.module.css";

type LiveSource = "off" | "radio" | "fc";

/** How often the FC's channels are read while live. */
const FC_POLL_MS = 1000;

export function SwitchesSegment({ d }: { d?: DeviceRef }) {
  const m = useSwitchMap();
  const [source, setSource] = useState<LiveSource>("off");
  const connected = useStore((s) => s.gear?.connected);
  const fcs = (connected ?? []).filter((c) => c.kind === "fc" && c.link.kind === "serial");
  const port = d?.connected?.link.kind === "serial" ? d.connected.link.port : fcs.length === 1 && fcs[0].link.kind === "serial" ? fcs[0].link.port : null;
  const radio = useRadio(source === "radio");
  const [fcLive, setFcLive] = useState<Live | null>(null);
  const [liveError, setLiveError] = useState<string | null>(null);

  useEffect(() => {
    if (source !== "fc" || !m.map) return;
    let gone = false;
    const read = () =>
      api.gearSwitchMap({ radio: m.src.radio, fc: m.src.fc, live: true, port }).then(
        (x) => {
          if (gone) return;
          setFcLive(x.live);
          setLiveError(null);
        },
        (e) => {
          if (!gone) setLiveError(errText(e));
        },
      );
    void read();
    const t = window.setInterval(read, FC_POLL_MS);
    return () => {
      gone = true;
      window.clearInterval(t);
      setFcLive(null);
    };
  }, [source, m.map, m.src, port]);

  const live = !m.map ? null : source === "fc" ? fcLive : source === "radio" && radio?.frame ? liveOf(m.map, "radio", radio.frame.channels) : null;

  return (
    <div className={styles.segment}>
      <MapSourceBar src={m.src} map={m.map} openCard={m.openCard} openModel={m.openModel} openDump={m.openDump} />
      {m.map && (
        <div className={styles.bar}>
          <SegmentedControl
            label="Live"
            size="sm"
            value={source}
            onChange={setSource}
            segments={[
              { value: "off", label: "Off" },
              { value: "radio", label: "Radio" },
              ...(port ? [{ value: "fc" as const, label: "FC" }] : []),
            ]}
          />
          {source === "radio" && radio && !radio.connected && <span className={styles.source}>No radio in USB Joystick mode.</span>}
        </div>
      )}
      {(m.error || liveError) && (
        <Banner kind="error" icon="danger-triangle" tint="red">
          {m.error || liveError}
        </Banner>
      )}
      {m.map ? <SwitchMapView map={m.map} live={live} /> : !m.error && <p className={styles.empty}>Open the radio's card or model file, and the FC's dump, to see what each switch does.</p>}
    </div>
  );
}
