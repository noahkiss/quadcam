// The segments of a device page. Later packages add theirs to this list (spec 2.2):
// an aircraft gets Switches, OSD, Rates, Settings, Backups and Changes; a radio Models,
// Voice, Checklists, Splash, Backups and Changes.
import { Overview } from "./Overview";
import type { DeviceRef, DeviceSegment } from "./slots";

export const DEVICE_SEGMENTS: DeviceSegment[] = [{ id: "overview", label: "Overview", kinds: "all", render: (d) => <Overview d={d} /> }];

/** The segments a device shows, in list order. */
export const segmentsFor = (d: Pick<DeviceRef, "kind">, list = DEVICE_SEGMENTS) => list.filter((s) => s.kinds === "all" || s.kinds.includes(d.kind));
