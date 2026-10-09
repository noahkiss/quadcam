// The mock core's Gear: the shapes of `gear_status`, `gear_devices` and the `device-changed`
// event, written by hand from `api/gear.rs` (a recorded `gear status` would hold the
// recording machine's devices). Behaviour follows `core/gear.rs` closely enough for the
// parity specs.
import type { Connected, Device, DeviceKind, GearSettings, GearStatus } from "../types";
import { HOME } from "./seed";
import { latestChecks, seedBackups, type MockBackup } from "./backups";
import type { CardCheck, CardMounted, GearJob, StepFailure } from "../types";
import { freshChanges, type MockChanges } from "./changes";

const GEAR_DIR = `${HOME}/Library/Application Support/app.quadcam/gear`;
const KINDS: DeviceKind[] = ["fc", "radio", "elrs_tx", "elrs_rx", "goggles", "dvr_card"];

/** `GearSettings::defaults`. */
export function gearDefaults(): GearSettings {
  return {
    gear_dir: GEAR_DIR,
    auto_backup: true,
    keep_recent: 10,
    keep_weeks: 8,
    keep_monthly: true,
    usb_minutes: 20,
    firmware_check: "manual",
    tts_provider: "say",
    on_connect: Object.fromEntries(KINDS.map((k) => [k, ["backup"]])),
    cues: {
      mute: false,
      speech: true,
      sound: false,
      notification: true,
      safe_to_unplug: true,
      still_inserted: true,
      step_failed: true,
      unplug_now: true,
      debounce_s: 30,
      reminder_grace_s: 60,
      still_inserted_every_s: 300,
      reminder_max: 3,
      quiet_hours: null,
      voice: null,
      voice_source: "macos",
    },
  };
}

/** `GearSettings::from_values`: the file keys over the defaults. */
export function gearSettings(values: Record<string, unknown>): GearSettings {
  const s = gearDefaults();
  const v = values as Record<string, never>;
  if (v.gearAutoBackup != null) s.auto_backup = v.gearAutoBackup;
  if (v.gearKeepRecent != null) s.keep_recent = v.gearKeepRecent;
  if (v.gearKeepWeeks != null) s.keep_weeks = v.gearKeepWeeks;
  if (v.gearKeepMonthly != null) s.keep_monthly = v.gearKeepMonthly;
  if (v.gearUsbMinutes != null) s.usb_minutes = v.gearUsbMinutes;
  if (v.gearOnConnect != null) s.on_connect = { ...s.on_connect, ...(v.gearOnConnect as object) };
  if (v.gearCues != null) s.cues = { ...s.cues, ...(v.gearCues as object) };
  return s;
}

export const volume = (mount: string, disk: string): Connected["link"] => ({ kind: "volume", mount, volume_uuid: null, bus_protocol: "USB", whole_disk: disk });

export const RADIO: Device = { id: "radio-1f2e3d4c5b6a7980", kind: "radio", name: "Field radio", aircraft: null, identity: { board: "tx16s", firmware: "EdgeTX", version: "2.11.2" }, last_seen: "2026-10-06T18:20:00Z", last_backup: "radio-1f2e3d4c5b6a7980/2026-10-06T182000-manual" };
export const FC: Device = { id: "fc-0a1b2c3d4e5f6071", kind: "fc", name: "Whoop FC", aircraft: "Whoop", identity: { board: "STM32F411", firmware: "Betaflight", version: "4.5.1", target: "BETAFPVF411" }, last_seen: "2026-10-05T10:00:00Z", last_backup: "fc-0a1b2c3d4e5f6071/2026-10-05T100000-before_apply" };

export const radioConnected = (): Connected => ({ id: RADIO.id, kind: "radio", link: volume("/Volumes/RADIO", "disk4"), identity: { board: "tx16s", version: "2.11.2" } });
export const dvrConnected = (): Connected => ({ id: "dvr-5a6b7c8d9e0f1a2b", kind: "dvr_card", link: volume("/Volumes/DVR", "disk5"), identity: {} });
export const gogglesConnected = (): Connected => ({ id: "goggles-77aa88bb99cc00dd", kind: "goggles", link: volume("/Volumes/DJI", "disk6"), identity: {} });

export interface MockGear {
  devices: Device[];
  connected: Connected[];
  working: string[];
  reminders: string[];
  /** FC ports whose background reads are paused. */
  paused: string[];
  /** Snapshots (`ipc/mock/backups.ts`). */
  backups: MockBackup[];
  /** Card checks, newest first. */
  checks: CardCheck[];
  /** Devices whose next card check fails. */
  cardFails: string[];
  /** The device whose next backup finds a change. */
  dirty: string | null;
  jobs: GearJob[];
  failures: StepFailure[];
  /** Staged changes (`ipc/mock/changes.ts`). */
  changeStore: MockChanges;
  /** Cards unmounted but still in (the `device-changed` event's list). */
  unmounted: Connected[];
  /** Cards the person mounted to browse. */
  mounted: CardMounted[];
  /** `._` files on the plugged-in radio card (`gear_card_clean` lists, then removes them). */
  appleDoubles: number;
}

/** No gear plugged in; two devices saved, with backups. */
export const quietGear = (): MockGear => ({ devices: [structuredClone(RADIO), structuredClone(FC)], connected: [], working: [], reminders: [], paused: [], backups: seedBackups(), checks: [], cardFails: [], dirty: null, jobs: [], failures: [], changeStore: freshChanges(), unmounted: [], mounted: [], appleDoubles: 0 });

/** A saved radio, a DVR card QuadCam does not know, and goggles a job is reading. */
export const busyGear = (): MockGear => ({ ...quietGear(), connected: [radioConnected(), dvrConnected(), gogglesConnected()], working: ["disk6"] });

/** `gear_status`, each connected device with its saved record. */
export function gearStatus(g: MockGear, values: Record<string, unknown>): GearStatus {
  return {
    gear_dir: GEAR_DIR,
    settings: gearSettings(values),
    connected: g.connected.map((c) => ({ ...c, device: g.devices.find((d) => d.id === c.id) || null })),
    devices: g.devices.length,
    staged: g.changeStore.changes.filter((c) => ["draft", "ready", "try", "read_first"].includes(c.status)).length,
    sims_out_of_date: 0,
    working: [...g.working],
    reminders: [...g.reminders],
    paused: [...g.paused],
    jobs: structuredClone(g.jobs),
    card_checks: latestChecks(g),
    failures: structuredClone(g.failures),
    mounted: structuredClone(g.mounted),
  };
}
