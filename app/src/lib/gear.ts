// Gear's words and states, shared by the sidebar, the status bar and the device pages.
// Pure functions over the core's shapes; no store, no calls.
import type { Connected, Device, DeviceKind, GearStatus, Link } from "../ipc/types";
import type { IconName } from "../components/Icon";

/** What the Connected page says when nothing is plugged in (`mcp::gear::NOTHING_FOUND`). */
export const NOTHING_FOUND = "Nothing found. If macOS asked to allow an accessory, click Allow.";

/** `DeviceKind::label` in the core. */
export const KIND_LABEL: Record<DeviceKind, string> = {
  fc: "FC",
  radio: "Radio",
  elrs_tx: "ELRS TX",
  elrs_rx: "ELRS RX",
  goggles: "Goggles",
  dvr_card: "DVR card",
};

/** The status bar's groups: one icon each. */
export type KindGroup = "radio" | "quad" | "dji" | "dvr";

export const GROUP_OF: Record<DeviceKind, KindGroup> = {
  radio: "radio",
  elrs_tx: "radio",
  fc: "quad",
  elrs_rx: "quad",
  goggles: "dji",
  dvr_card: "dvr",
};

export const GROUPS: { id: KindGroup; label: string; icon: IconName }[] = [
  { id: "radio", label: "Radio", icon: "radio" },
  { id: "quad", label: "Quad", icon: "quad" },
  { id: "dji", label: "DJI", icon: "goggles" },
  { id: "dvr", label: "DVR card", icon: "sd-card" },
];

export const KIND_ICON: Record<DeviceKind, IconName> = {
  radio: "radio",
  elrs_tx: "radio",
  fc: "quad",
  elrs_rx: "quad",
  goggles: "goggles",
  dvr_card: "sd-card",
};

/** `core::link_handle`: what stays the same for a device's link across a mount and an
 *  unmount. Holds and reminders are keyed by it. */
export function linkHandle(link: Link): string {
  switch (link.kind) {
    case "volume":
      return link.whole_disk || link.mount;
    case "serial":
      return link.port;
    case "dfu":
      return `dfu-${hex4(link.vid)}:${hex4(link.pid)}`;
  }
}
const hex4 = (n: number) => n.toString(16).padStart(4, "0");

/** `Device::display_name`. */
export const deviceName = (d: Pick<Device, "name" | "kind">) => (d.name?.trim() ? d.name : `Unnamed ${KIND_LABEL[d.kind]}`);

/** The name for a connected device: its saved name, else its kind. */
export const connectedName = (c: Connected) => (c.device ? deviceName(c.device) : KIND_LABEL[c.kind]);

/** A device page's key: the device id, else the link (a device QuadCam cannot name yet). */
export const pageKey = (c: Connected) => c.id || `link:${linkHandle(c.link)}`;

/** Where the device is reached: its mount point or serial port. */
export function linkText(link: Link): string {
  switch (link.kind) {
    case "volume":
      return link.mount;
    case "serial":
      return link.port;
    case "dfu":
      return `DFU ${hex4(link.vid)}:${hex4(link.pid)}`;
  }
}

/** A connected device's state, most urgent first. */
export type DeviceState = "attention" | "inserted" | "working" | "safe" | "connected";

export const STATE_LABEL: Record<DeviceState, string> = {
  attention: "Needs attention",
  inserted: "Still inserted",
  working: "Working",
  safe: "Safe to unplug",
  connected: "Connected",
};

const RANK: DeviceState[] = ["attention", "inserted", "working", "safe", "connected"];

/** The more urgent of two states. */
export const worst = (a: DeviceState, b: DeviceState) => (RANK.indexOf(a) <= RANK.indexOf(b) ? a : b);

export interface StateInput {
  status: Pick<GearStatus, "working" | "reminders"> | null;
  /** When each unmounted link was first seen unmounted (ms). */
  unmountedSince: Record<string, number>;
  /** Seconds after "done" before the reminder ("still inserted"). */
  graceS: number;
  now: number;
}

/** What a device is doing: a job holds it (working); unmounted but still in (safe to
 *  unplug, then still inserted once its reminder is due); not saved yet (needs
 *  attention); else connected. */
export function deviceState(c: Connected, unmounted: boolean, x: StateInput): DeviceState {
  const h = linkHandle(c.link);
  if (x.status?.working?.includes(h)) return "working";
  if (unmounted) {
    const since = x.unmountedSince[h] ?? x.now;
    const armed = x.status?.reminders?.includes(h);
    return armed && x.now - since >= x.graceS * 1000 ? "inserted" : "safe";
  }
  if (!c.device) return "attention";
  return "connected";
}

/** What a needs-attention device lacks, in a word. */
export const attentionText = (c: Connected) => (c.id ? "Not saved" : "Not identified");
