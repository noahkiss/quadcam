// The segments of a device page. Later packages add theirs to this list (spec 2.2):
// an aircraft gets Switches, OSD, Rates, Settings, Backups and Changes; a radio Models,
// Voice, Checklists, Splash, Backups and Changes.
import { Overview } from "./Overview";
import { OsdSegment } from "./Osd/OsdSegment";
import { RatesSegment } from "./Rates/RatesSegment";
import { SwitchesSegment } from "./Switches/SwitchesSegment";
import { ChangesSegment } from "./Changes/ChangesSegment";
import { BackupsSegment } from "./Backups/BackupsSegment";
import { SplashSegment } from "./Splash/SplashSegment";
import type { DeviceRef, DeviceSegment } from "./slots";

export const DEVICE_SEGMENTS: DeviceSegment[] = [
  { id: "overview", label: "Overview", kinds: "all", render: (d) => <Overview d={d} /> },
  // The FC's latest backup, or a dump or diff file.
  { id: "osd", label: "OSD", kinds: ["fc"], render: (d) => <OsdSegment key={d.key} device={d.device?.last_backup ? d.device.id : null} /> },
  // Every rate profile of the FC's latest backup, or of a dump file, with the sims' rates.
  { id: "rates", label: "Rates", kinds: ["fc"], render: (d) => <RatesSegment key={d.key} device={d.device?.last_backup ? d.device.id : null} /> },
  // From a card or model file and a dump; the latest backups join as sources later.
  { id: "switches", label: "Switches", kinds: ["fc", "radio"], render: (d) => <SwitchesSegment d={d} /> },
  // Staged changes, the history, and Edit setting (WP5).
  { id: "changes", label: "Changes", kinds: ["fc", "radio"], render: (d) => <ChangesSegment d={d} /> },
  // The start-up picture, patched into the radio's firmware (WP10).
  { id: "splash", label: "Splash", kinds: ["radio"], render: (d) => <SplashSegment key={d.key} d={d} /> },
  { id: "backups", label: "Backups", kinds: ["radio", "fc", "goggles", "dvr_card"], render: (d) => <BackupsSegment d={d} /> },
];

/** The segments a device shows, in list order. */
export const segmentsFor = (d: Pick<DeviceRef, "kind">, list = DEVICE_SEGMENTS) => list.filter((s) => s.kinds === "all" || s.kinds.includes(d.kind));
