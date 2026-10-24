import { useEffect, useState } from "react";
import { useStore } from "../../store";
import { api, errText } from "../../ipc/api";
import { toast } from "../../components/toastStore";
import type { BlackboxEntry } from "../../ipc/types";
import { Button } from "../../components/Button";
import { FactList, type Fact } from "../../components/dash/FactList";
import { SectionCard } from "../../components/dash/SectionCard";
import { Stat, StatRow } from "../../components/dash/StatRow";
import { PlugInBar } from "../../components/gear/PlugInBar";
import { checkFor, fmtWhen } from "../../lib/backups";
import { linkHandle, linkText } from "../../lib/gear";
import { aircraftOf } from "../../lib/radioAircraft";
import { fmtAgo, fmtBytes, plural } from "../../lib/format";
import { gearSlots, type DeviceRef } from "./slots";
import { segmentsFor } from "./segments";
import { RadioAircraft } from "./RadioAircraft/RadioAircraft";
import styles from "./Gear.module.css";
import { PathText } from "../../components/PathText";

const IDENTITY: [keyof NonNullable<DeviceRef["connected"]>["identity"], string][] = [
  ["board", "Board"],
  ["firmware", "Firmware"],
  ["version", "Version"],
  ["build", "Build"],
  ["target", "Target"],
];

/** The identity a device page shows: the saved one, with what the device reports now on top. */
export const identityOf = (d: DeviceRef) => ({ ...(d.device?.identity || {}), ...Object.fromEntries(Object.entries(d.connected?.identity || {}).filter(([, v]) => v)) });

/** A device's Overview: the bar for staged changes, the summary tiles for its kind, and its
 *  details (identity, where it is plugged in, its aircraft); a radio adds
 *  its list of aircraft. */
export function Overview({ d }: { d: DeviceRef }) {
  useStore((s) => s.changes); // the bar and the Staged tile follow the staged changes
  const setFilter = useStore((s) => s.setFilter);
  const paused = useStore((s) => s.gear?.paused);
  const setPollPaused = useStore((s) => s.setPollPaused);
  const identity = identityOf(d);
  const aircraft = d.device?.aircraft;
  const isRadio = d.kind === "radio";
  const fcPort = d.connected?.kind === "fc" && d.connected.link.kind === "serial" ? linkHandle(d.connected.link) : null;
  const isPaused = fcPort !== null && !!paused?.includes(fcPort);
  const facts: Fact[] = IDENTITY.filter(([k]) => identity[k]).map(([k, label]) => ({ label, value: identity[k], selectable: true }));
  if (d.connected) facts.push({ label: d.connected.link.kind === "volume" ? "Mounted at" : "Port", value: d.unmounted ? "Unmounted" : d.connected.link.kind === "dfu" ? linkText(d.connected.link) : <PathText path={linkText(d.connected.link)} />, selectable: !d.unmounted });
  if (fcPort)
    facts.push({
      label: "Background reads",
      value: (
        <>
          {isPaused ? "Paused" : "On"}
          <Button size="sm" variant="ghost" onClick={() => setPollPaused(fcPort, !isPaused)}>
            {isPaused ? "Resume Reads" : "Pause Reads"}
          </Button>
        </>
      ),
    });
  if (!isRadio)
    facts.push({
      label: "Aircraft",
      value: (
        <>
          {aircraft || "None"}
          {aircraft && (
            <Button size="sm" variant="ghost" onClick={() => setFilter({ group: "all", aircraft })}>
              Show Clips
            </Button>
          )}
        </>
      ),
    });
  const source = useFirmwareSource(d);
  if (source) facts.push({ label: "Firmware source", value: source });
  if (d.device?.last_seen) facts.push({ label: "Last seen", value: fmtWhen(d.device.last_seen) });
  return (
    <div className={styles.overview}>
      {d.device && <PlugInBar count={gearSlots.stagedFor(d.device.id)} onReview={() => gearSlots.review(d.device!.id)} />}
      <Tiles d={d} />
      <SectionCard title="Details">
        <FactList facts={facts} label="Details" />
      </SectionCard>
      {isRadio && d.device && <RadioAircraft d={d} />}
    </div>
  );
}

/** The summary tiles for a device's kind. A tile that sums up a segment opens it. */
function Tiles({ d }: { d: DeviceRef }) {
  const firmware = useStore((s) => s.firmware);
  const saved = useStore((s) => s.devices);
  const gear = useStore((s) => s.gear);
  const cards = useStore((s) => s.cards);
  const setSegment = useStore((s) => s.setGearSegment);
  const openGear = useStore((s) => s.openGear);
  const pulls = useBlackbox(d.kind === "fc" ? (d.device?.id ?? null) : null);
  const ids = new Set(segmentsFor(d).map((s) => s.id));
  const go = (seg: string) => (ids.has(seg) ? () => setSegment(seg) : undefined);
  const id = d.device?.id ?? d.connected?.id ?? null;
  const backup = gearSlots.lastBackup(d.device);
  const staged = d.device ? gearSlots.stagedFor(d.device.id) : 0;

  const fw = firmware?.devices.find((f) => f.device === id);
  const installed = fw?.installed ?? identityOf(d).version ?? null;
  const fwTile = (
    <Stat
      key="firmware"
      label="Firmware"
      value={installed ?? "—"}
      sub={!fw?.latest ? "Newest not checked" : fw.state === "update" ? `Newest ${fw.latest}` : "Up to date"}
      tone={fw?.state === "update" ? "warn" : undefined}
      onClick={() => openGear({ page: "slot", id: "firmware" })}
    />
  );
  const backupTile = <Stat key="backup" label="Last backup" value={fmtAgo(backup)} sub={backup ? fmtWhen(backup) : "None"} onClick={go("backups")} />;
  const stagedTile = <Stat key="staged" label="Staged" value={staged} sub={staged ? "Ready to apply" : undefined} onClick={go("changes")} />;

  let tiles;
  if (d.kind === "fc") {
    const last = pulls?.[0]?.pull;
    tiles = [
      fwTile,
      backupTile,
      stagedTile,
      <Stat key="blackbox" label="Blackbox" value={last ? plural(last.logs.length, "log") : "—"} sub={last ? fmtWhen(last.pulled_at) : "Not pulled"} onClick={go("blackbox")} />,
    ];
  } else if (d.kind === "radio") {
    const n = aircraftOf(d.device, saved).length;
    tiles = [fwTile, backupTile, stagedTile, <Stat key="aircraft" label="Aircraft" value={n} />];
  } else if (d.kind === "dvr_card" || d.kind === "goggles") {
    const mount = d.connected?.link.kind === "volume" && !d.unmounted ? d.connected.link.mount : null;
    const card = mount ? cards[mount] : undefined;
    const check = checkFor(gear, id);
    const space = d.device?.last_space;
    const checkWord = check ? { ok: "Passed", failed: "Problems", stopped: "Stopped", error: "Did not run" }[check.state] : "—";
    tiles = [
      <Stat key="new" label="New clips" value={card ? card.new : "—"} sub={card ? plural(card.clips, "clip") + " on the card" : undefined} />,
      <Stat key="check" label="Last check" value={checkWord} sub={check ? fmtWhen(check.at) : "Not checked"} tone={check ? (check.state === "ok" ? "ok" : check.state === "stopped" ? undefined : "error") : undefined} />,
      <Stat
        key="space"
        label="Free space"
        value={space ? fmtBytes(space.free) : "—"}
        sub={space?.total ? `of ${fmtBytes(space.total)}` : undefined}
        meter={space?.total ? 1 - space.free / space.total : undefined}
      />,
      backupTile,
    ];
  } else {
    tiles = [fwTile, backupTile, stagedTile];
  }
  return <StatRow label="Summary">{tiles}</StatRow>;
}

/** An FC's firmware source picker, shown when QuadCam knows its maker's builds: the maker,
 *  official Betaflight, or what QuadCam detects. */
function useFirmwareSource(d: DeviceRef) {
  const firmware = useStore((s) => s.firmware);
  const load = useStore((s) => s.loadFirmware);
  const id = d.device?.id ?? null;
  const isFc = d.kind === "fc" && !!id;
  useEffect(() => {
    if (isFc && !firmware) void load(false);
  }, [isFc, firmware, load]);
  const fw = firmware?.devices.find((f) => f.device === id);
  if (!isFc || !fw?.vendor) return null;
  const chosen = d.device?.firmware_channel ?? "";
  const label = (c: string | null | undefined) => (c === "vendor" ? fw.vendor : "Official Betaflight");
  const pick = async (v: string) => {
    try {
      await api.gearDeviceSave(id!, null, null, (v || "auto") as "vendor" | "official" | "auto");
      await load(false);
    } catch (e) {
      toast(errText(e), true);
    }
  };
  return (
    <select aria-label="Firmware source" value={chosen} onChange={(e) => void pick(e.target.value)}>
      <option value="">{chosen ? "Automatic" : `Automatic (${label(fw.channel)})`}</option>
      <option value="vendor">{fw.vendor}</option>
      <option value="official">Official Betaflight</option>
    </select>
  );
}

/** The FC's stored blackbox pulls, newest first; null until they load. */
function useBlackbox(device: string | null) {
  const [list, setList] = useState<BlackboxEntry[] | null>(null);
  useEffect(() => {
    if (!device) return;
    let gone = false;
    api.gearBlackbox(device).then(
      (l) => !gone && setList([...l].sort((a, b) => b.pull.pulled_at.localeCompare(a.pull.pulled_at))),
      () => undefined,
    );
    return () => {
      gone = true;
    };
  }, [device]);
  return device ? list : null;
}
