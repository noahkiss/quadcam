import { useStore } from "../../store";
import { Button } from "../../components/Button";
import { PlugInBar } from "../../components/gear/PlugInBar";
import { linkHandle, linkText } from "../../lib/gear";
import { tilde } from "../../lib/format";
import { gearSlots, type DeviceRef } from "./slots";
import { segmentsFor } from "./segments";
import { RadioAircraft } from "./RadioAircraft/RadioAircraft";
import styles from "./Gear.module.css";

const IDENTITY: [keyof NonNullable<DeviceRef["connected"]>["identity"], string][] = [
  ["board", "Board"],
  ["firmware", "Firmware"],
  ["version", "Version"],
  ["build", "Build"],
  ["target", "Target"],
];

const fmtTime = (t: string | null | undefined) => (t ? new Date(t).toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" }) : null);

/** A device's Overview: the bar for staged changes, its identity, where it is plugged in,
 *  its aircraft (a radio's list of them) and its latest backup, and the device's other
 *  segments. */
export function Overview({ d }: { d: DeviceRef }) {
  const home = useStore((s) => s.home);
  useStore((s) => s.changes); // the bar follows the staged changes
  const setFilter = useStore((s) => s.setFilter);
  const setSegment = useStore((s) => s.setGearSegment);
  const identity = { ...(d.device?.identity || {}), ...Object.fromEntries(Object.entries(d.connected?.identity || {}).filter(([, v]) => v)) };
  const shown = IDENTITY.filter(([k]) => identity[k]);
  const backup = gearSlots.lastBackup(d.device);
  const others = segmentsFor(d).filter((s) => s.id !== "overview");
  const aircraft = d.device?.aircraft;
  const isRadio = d.kind === "radio";
  const paused = useStore((s) => s.gear?.paused);
  const setPollPaused = useStore((s) => s.setPollPaused);
  const fcPort = d.connected?.kind === "fc" && d.connected.link.kind === "serial" ? linkHandle(d.connected.link) : null;
  const isPaused = fcPort !== null && !!paused?.includes(fcPort);
  return (
    <div className={styles.overview}>
      {d.device && <PlugInBar count={gearSlots.stagedFor(d.device.id)} onReview={() => gearSlots.review(d.device!.id)} />}
      <dl className={styles.facts}>
        {shown.map(([k, label]) => (
          <div key={k}>
            <dt>{label}</dt>
            <dd className="selectable">{identity[k]}</dd>
          </div>
        ))}
        {d.connected && (
          <div>
            <dt>{d.connected.link.kind === "volume" ? "Mounted at" : "Port"}</dt>
            <dd className="selectable">{d.unmounted ? "Unmounted" : tilde(linkText(d.connected.link), home)}</dd>
          </div>
        )}
        {fcPort && (
          <div>
            <dt>Background reads</dt>
            <dd>
              {isPaused ? "Paused" : "On"}
              <Button size="sm" variant="ghost" onClick={() => setPollPaused(fcPort, !isPaused)}>
                {isPaused ? "Resume reads" : "Pause reads"}
              </Button>
            </dd>
          </div>
        )}
        {!isRadio && (
          <div>
            <dt>Aircraft</dt>
            <dd>
              {aircraft || "None"}
              {aircraft && (
                <Button size="sm" variant="ghost" onClick={() => setFilter({ group: "all", aircraft })}>
                  Show clips
                </Button>
              )}
            </dd>
          </div>
        )}
        <div>
          <dt>Last backup</dt>
          <dd>{fmtTime(backup) || "None"}</dd>
        </div>
        {d.device?.last_seen && (
          <div>
            <dt>Last seen</dt>
            <dd>{fmtTime(d.device.last_seen)}</dd>
          </div>
        )}
      </dl>
      {isRadio && d.device && <RadioAircraft d={d} />}
      {others.length > 0 && (
        <nav aria-label="Sections" className={styles.sections}>
          {others.map((s) => (
            <Button key={s.id} size="sm" variant="ghost" iconEnd="chev-right" onClick={() => setSegment(s.id)}>
              {s.label}
            </Button>
          ))}
        </nav>
      )}
    </div>
  );
}
