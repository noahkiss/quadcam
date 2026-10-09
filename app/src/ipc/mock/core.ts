// A fake core for tests and design work: answers the Tauri commands the UI uses, keeps
// state in memory, and emits the events the real core emits (`library-changed`,
// `session-changed`, `settings-changed`, progress). Shapes follow the recorded fixtures;
// behaviour follows `Core` closely enough for the parity specs.
import type {
  ModuleStatus,
  ClipPlan,
  ClipResult,
  CutChange,
  GeoResult,
  LibClip,
  LibCut,
  LibEdit,
  LibraryView,
  Moved,
  PlanPatch,
  Session,
  SettingsView,
  Span,
  Volume,
} from "../types";
import * as seed from "./seed";
import * as gear from "./gear";
import { MockFlights } from "./flights";
import * as backups from "./backups";
import { MockSim, defaults as simDefaults } from "./sim";
import { location as normLocation, spans as normSpans } from "../normalize";
import { live as liveOf } from "../../lib/controls";

/** `Core::dispatch` methods, each also a typed Tauri command of the same name. */
const DISPATCH = new Set([
  "library", "library_rate", "library_edit", "library_rename", "library_cuts", "library_export_cuts", "library_trash", "library_untrash",
  "library_photos", "library_apply_name_format", "library_match_logs", "library_rebuild", "library_rescan", "library_preview", "library_strips", "card_status",
  "settings", "settings_set", "place_search", "place_save", "session_cuts", "profiles", "session_split", "library_split",
  "modules", "module_install", "module_remove", "modules_check", "gear_osd",
  "gear_status", "gear_devices", "gear_device_save", "gear_device_forget", "gear_dismiss_reminder", "gear_poll_pause",
  "gear_switch_map", "gear_radio", "gear_radio_watch", "gear_sim_calibration", "gear_sim_calibration_save", "gear_sim_defaults", "gear_sim_calibrate",
  "gear_flights", "gear_flight_set", "gear_flight_folders", "gear_packs", "gear_pack_save", "gear_pack_delete", "gear_pack_type_save",
  "gear_pack_type_delete", "gear_pack_notes", "gear_session_report", "gear_preflight", "gear_crashes", "gear_crash_save", "gear_crash_delete",
  "gear_backup", "gear_backups", "gear_backup_read", "gear_backup_diff", "gear_backup_pin", "gear_storage", "gear_prune",
  "gear_export", "gear_import_backups", "gear_card_check", "gear_card_checks", "gear_card_repair", "gear_stop",
]);

export type Scenario = "library" | "empty" | "card" | "review" | "joined" | "finished-card" | "dji" | "no-tools" | "many" | "gear";

export interface MockOptions {
  scenario?: Scenario;
  /** Milliseconds each call waits before it answers. */
  latency?: number;
}

export type Emit = (event: string, payload?: unknown) => void;

export interface Call {
  cmd: string;
  args: Record<string, unknown>;
}

const sameSpan = (a: Span, b: Span) => Math.abs(a.start - b.start) < 0.001 && Math.abs(a.end - b.end) < 0.001;
const base = (p: string) => p.split("/").pop() || p;
const dirOf = (p: string) => p.split("/").slice(0, -1).join("/");
const slug = (s: string) => s.trim().replace(/\s+/g, "-").replace(/[/:]/g, "-");

export class MockCore {
  lib: LibraryView;
  session: Session | null;
  settings: SettingsView;
  /** The settings values the mock started with. */
  initialValues: SettingsView["values"];
  volumes: Volume[];
  tools = true;
  modules: ModuleStatus[] = seed.modules();
  gear: gear.MockGear = gear.quietGear();
  /** The Gear seed helpers, for specs (`core.gearSeed.radioConnected()`). */
  gearSeed = gear;
  /** Flights, packs and crashes (`./flights.ts`). */
  flights = new MockFlights();
  trash = new Map<string, LibClip>();
  calls: Call[] = [];
  menuState: unknown = null;
  /** Answers for the folder picker, used in order; then null. */
  dialogAnswers: (string | string[] | null)[] = [];
  /** The radio in USB Joystick mode: plugged in or not, its latest frame, and whether the
   *  page streams it (`gear_radio_watch`). Specs move it with `radioFrame`. */
  radio: { connected: boolean; frame: import("../types").RadioFrame | null; watching: boolean } = { connected: false, frame: null, watching: false };
  /** The sim's calibration (`./sim.ts`). */
  sim = new MockSim();
  /** What the FC's `MSP_RC` reports (µs, CH1 first). */
  fcRc: number[] = [1500, 1500, 988, 1500, 988, 988, 988, 988];
  /** Answers given to agent format requests. */
  formatAnswers: { id: number; approve: boolean }[] = [];
  private emit: Emit;

  constructor(emit: Emit, opts: MockOptions = {}) {
    this.emit = emit;
    const sc = opts.scenario || "library";
    this.lib = sc === "empty" || sc === "no-tools" ? seed.emptyLibrary() : seed.richLibrary();
    if (sc === "many") this.lib = seed.recount(manyClips(this.lib));
    this.settings = seed.settings();
    this.initialValues = structuredClone(this.settings.values);
    this.volumes = sc === "card" || sc === "finished-card" ? [seed.cardVolume()] : [];
    this.session = null;
    if (sc === "review") this.session = seed.reviewSession();
    if (sc === "joined") this.session = seed.joinedSession();
    if (sc === "dji") this.volumes = [seed.djiVolume()];
    if (sc === "finished-card" || sc === "dji") {
      this.session = seed.finishedSession();
      const v = sc === "dji" ? seed.djiVolume() : seed.cardVolume();
      this.session.source = v.mount;
      if (v.source === "dji") asDji(this.session);
      this.session.card_volume = v;
      this.session.card = {
        device_identifier: v.info.device_identifier,
        whole_disk: v.info.parent_whole_disk,
        volume_uuid: v.info.volume_uuid,
        volume_name: v.info.volume_name,
        total_size: v.info.total_size,
        media_name: v.info.media_name,
      };
    }
    if (sc === "no-tools") this.tools = false;
    if (sc === "gear") this.gear = gear.busyGear();
  }

  // ---------- the Tauri commands ----------

  handle(cmd: string, args: Record<string, unknown>): unknown {
    // The typed commands (R5) take one `params` struct and share the dispatch names.
    const p = (args.params ?? {}) as Record<string, unknown>;
    switch (cmd) {
      case "volumes":
        return structuredClone(this.volumes);
      case "session":
        return structuredClone(this.session);
      case "load":
        return this.load(String(p.source));
      case "dates": {
        const logs = p.logs as { kind: string; path?: string } | undefined;
        return this.planDates(logs?.kind === "dir" ? logs.path! : null, (p.day as string | null) ?? null);
      }
      case "suggest":
        return this.patch(p.patches as PlanPatch[]);
      case "import":
        return this.importClips(p as { output_dir: string; format: string; keep_clips?: boolean });
      case "photos":
        return this.addToPhotos((p.ids as number[] | null) ?? null, (p.album as string | null) ?? null);
      case "clear":
        this.session = null;
        return { cleared: true };
      case "eject":
        this.volumes = this.volumes.filter((v) => !v.is_card);
        this.emit("volumes-changed");
        return { ejected: true };
      case "format_plan":
        return this.formatPlan((p.label as string | null) ?? null);
    }
    if (DISPATCH.has(cmd)) return this.dispatch(cmd, p);
    switch (cmd) {
      case "env_check":
        return this.tools
          ? { tools: this.ffmpegTools(), error: null, install_hint: "brew install ffmpeg", socket: `${seed.HOME}/Library/Application Support/app.quadcam/control.sock`, ...seed.VERSION }
          : { tools: null, error: "ffmpeg and ffprobe not found.", install_hint: "brew install ffmpeg", socket: null, ...seed.VERSION };
      case "default_output_dir":
        return seed.LIBRARY_ROOT;
      case "load_dropped":
        return this.load((args.paths as string[])[0] ?? "dropped");
      case "format_card":
        return this.formatCard(String(args.label));
      case "answer_format_request":
        this.formatAnswers.push({ id: Number(args.id), approve: !!args.approve });
        return null;
      case "preview":
        return seed.PREVIEW;
      case "library_scope":
      case "share":
        return null;
      case "menu_state":
        this.menuState = args;
        return null;
      case "plugin:dialog|open":
        return this.dialogAnswers.length ? this.dialogAnswers.shift() : null;
      case "plugin:opener|reveal_item_in_dir":
      case "plugin:opener|open_path":
      case "plugin:opener|open_url":
        return null;
      case "third_party_notices":
        return seed.NOTICES;
      default:
        throw `mock core: no command ${cmd}`;
    }
  }

  // ---------- Core::dispatch ----------

  dispatch(method: string, p: Record<string, unknown>): unknown {
    switch (method) {
      case "library":
        return this.libraryView();
      case "library_rate":
        return this.rate(p.ids as string[], p.rating as number | null, p.flag as LibClip["flag"] | null);
      case "library_edit": {
        const { id, ...edit } = p as unknown as { id: string } & LibEdit;
        return this.edit(id, edit);
      }
      case "library_rename":
        return this.rename(String(p.id), String(p.name));
      case "library_cuts":
        return this.libraryCuts(String(p.id), p.cuts as Span[], p.removed_cuts as string | null);
      case "library_export_cuts":
        return this.exportCuts(String(p.id));
      case "library_trash":
        return this.trashClips(p.ids as string[]);
      case "library_untrash":
        return this.untrash(p.moved as Moved[]);
      case "library_photos":
        return this.libraryPhotos(p.ids as string[], (p.album as string) ?? "");
      case "library_apply_name_format":
        return { renamed: [], unchanged: this.lib.clips.length, skipped: [], failed: [] };
      case "library_match_logs":
        return {
          clips: this.lib.clips.map((c) => ({ id: c.id, path: c.path, duration: c.duration, badge: "unmatched", log_day: null, log_date: null, log_time: null, log_model: null, flights: 0, reason: null, flight: null, moments: 0, applied: false })),
          warnings: [],
        };
      case "library_rebuild":
        this.emit("library-task", { task: "rebuild", done: 0, total: 1 });
        this.emit("library-task", { task: "rebuild", done: 1, total: 1 });
        return { clips: this.lib.clips.length, cuts: this.lib.clips.reduce((a, c) => a + c.cuts.length, 0), problems: [] };
      case "library_rescan":
        return this.clip(String(p.id));
      case "library_preview":
        this.clip(String(p.id));
        return seed.PREVIEW;
      case "library_strips":
        return this.strips();
      case "card_status":
        return { mount: p.mount, clips: 5, new: this.session?.results.length ? 0 : 4, free: 30_000_000_000, size: 31_914_983_424 };
      case "settings":
        return structuredClone(this.settings);
      case "settings_set":
        return this.settingsSet(p.values as Record<string, unknown>);
      case "place_search":
        return this.placeSearch(String(p.query));
      case "place_save":
        return this.placeSave(String(p.name), Number(p.lat), Number(p.lon));
      case "session_cuts":
        return this.sessionCuts(Number(p.id), p.cuts as Span[], p.removed_cuts as string | null);
      case "session_split": {
        const id = Number(p.id);
        const add = flightCuts(this.plan(id).flight?.flight_spans || [], this.need().clips.find((c) => c.id === id)!.duration);
        return this.sessionCuts(id, withSpans(this.plan(id).cuts, add), null);
      }
      case "library_split": {
        const c = this.clip(String(p.id));
        const add = flightCuts(c.stats?.flight_spans || [], c.duration);
        return this.libraryCuts(c.id, withSpans([...c.cuts, ...c.pending_cuts], add), null);
      }
      case "profiles":
        return { profiles: this.settings.values.profiles || [], default_profile: this.settings.values.defaultProfile || null };
      case "gear_status":
        return gear.gearStatus(this.gear, this.settings.values);
      case "gear_devices":
        return structuredClone(this.gear.devices);
      case "gear_device_save":
        return this.deviceSave(String(p.id), (p.name as string | null) ?? null, (p.aircraft as string | null) ?? null);
      case "gear_device_forget": {
        const d = this.gear.devices.find((x) => x.id === p.id);
        if (!d) throw `No device ${p.id}.`;
        this.gear.devices = this.gear.devices.filter((x) => x !== d);
        this.emit("gear-changed");
        return d;
      }
      case "gear_poll_pause": {
        const port = String(p.port);
        this.gear.paused = this.gear.paused.filter((x) => x !== port);
        if (p.paused) this.gear.paused.push(port);
        this.emit("gear-changed");
        return [...this.gear.paused];
      }
      case "gear_dismiss_reminder": {
        const was = this.gear.reminders.includes(String(p.handle));
        this.gear.reminders = this.gear.reminders.filter((h) => h !== p.handle);
        if (was) this.emit("gear-changed");
        return was;
      }
      case "gear_switch_map": {
        const fc = (p.fc as string[] | undefined) ?? [];
        const devs = (p.devices as string[] | undefined) ?? [];
        if (!p.radio && !fc.length && !p.aircraft && !devs.length) throw "Pass an EdgeTX card or model file, a Betaflight dump, a device or an aircraft.";
        const m = seed.switchMap();
        const channels = (p.channels as number[] | undefined) ?? [];
        if (channels.length) m.live = liveOf(m, "given", channels);
        else if (p.live && this.gear.connected.some((c) => c.kind === "fc" && c.link.kind === "serial")) m.live = liveOf(m, "fc", this.fcRc);
        else if (p.live) {
          if (!this.radio.frame) throw "Nothing to read live: no FC is plugged in, and no radio is in USB Joystick mode.";
          m.live = liveOf(m, "radio", this.radio.frame.channels);
        }
        return m;
      }
      case "gear_radio":
        return this.radio.connected
          ? { connected: true, product: "Test Radio Joystick", frame: this.radio.frame, message: this.radio.frame ? null : "The radio sent no report: is USB Joystick mode on?" }
          : { connected: false, product: null, frame: null, message: "No radio in USB Joystick mode. Plug it in and choose USB Joystick on the radio." };
      case "gear_sim_calibration":
        return this.sim.calibration(this.radio.connected, this.gear.devices, (p.radio as string | null) ?? null);
      case "gear_sim_calibration_save":
        return this.sim.save(p as never);
      case "gear_sim_defaults":
        return simDefaults((p.aircraft as string | null) ?? null);
      case "gear_sim_calibrate": {
        const v = this.sim.calibrate(p as never);
        this.emit("sim-calibration-event", v);
        return v;
      }
      case "gear_radio_watch":
        this.radio.watching = !!p.on;
        if (this.radio.watching) this.emitRadio();
        return this.radio.watching;
      case "gear_backup":
        return this.gearBackup(p);
      case "gear_backups":
        return backups.list(this.gear, (p.device as string | null) ?? null);
      case "gear_backup_read":
        return backups.read(this.gear, String(p.id), (p.path as string | null) ?? null);
      case "gear_backup_diff":
        return backups.diff(this.gear, String(p.a), (p.b as string | null) ?? null);
      case "gear_backup_pin":
        return backups.pin(this.gear, String(p.id), !!p.pinned);
      case "gear_storage":
        return backups.storage(this.gear, gear.gearStatus(this.gear, this.settings.values).gear_dir);
      case "gear_prune": {
        const r = backups.prune(this.gear, !!p.dry_run);
        if (!p.dry_run) this.emit("gear-changed");
        return r;
      }
      case "gear_export": {
        const ids = p.snapshot ? [String(p.snapshot)] : backups.list(this.gear, String(p.device)).map((b) => b.id);
        if (!ids.length) throw "That device has no backups.";
        return { folders: ids.map((id) => `${p.to}/${id}`), files: ids.length * 3, bytes: ids.length * 4096 };
      }
      case "gear_import_backups": {
        const r = backups.importFolder(this.gear, String(p.folder), !!p.dry_run);
        if (!p.dry_run) this.emit("gear-changed");
        return r;
      }
      case "gear_card_check": {
        const c = backups.cardCheck(this.gear, String(p.device), new Date().toISOString());
        this.emit("gear-changed");
        return c;
      }
      case "gear_card_checks":
        return this.gear.checks.filter((c) => c.device === p.device);
      case "gear_card_repair": {
        if (!p.confirm) throw "Refused: a repair writes the card's file system; it needs confirm=true.";
        const r = backups.repair(this.gear, String(p.check), new Date().toISOString());
        this.emit("gear-changed");
        return r;
      }
      case "gear_stop":
        return this.gear.jobs.some((j) => j.handle === p.handle);
      case "gear_osd": {
        const paths = (p.paths as string[] | undefined) ?? [];
        const dev = this.gear.devices.find((d) => d.id === p.device);
        if (!paths.length && !dev?.last_backup) throw "Pass a Betaflight dump or diff file, or a device.";
        const v = seed.osd();
        v.source = paths.length ? paths.map(base) : [`${dev!.last_backup} dump all`];
        return v;
      }
      case "gear_flights":
        return this.flights.flights((p.day as string | null) ?? null);
      case "gear_flight_set":
        return this.gearChanged(this.flights.flightSet(String(p.flight), (p.pack as string | null) ?? null));
      case "gear_flight_folders":
        if (p.add) this.flights.folders.push(String(p.add));
        if (p.remove) this.flights.folders = this.flights.folders.filter((f) => f !== p.remove);
        return this.gearChanged([...this.flights.folders]);
      case "gear_packs":
        return this.flights.packsView();
      case "gear_pack_save":
        return this.gearChanged(this.flights.packSave(p.pack as import("../types").Pack, (p.charged as boolean | null) ?? null));
      case "gear_pack_delete":
        return this.gearChanged(this.flights.packDelete(String(p.name)));
      case "gear_pack_type_save":
        return this.gearChanged(this.flights.typeSave(p as unknown as import("../types").PackType));
      case "gear_pack_type_delete":
        return this.gearChanged(this.flights.typeDelete(String(p.name)));
      case "gear_pack_notes":
        this.flights.notes = String(p.text);
        return this.gearChanged(this.flights.notes);
      case "gear_session_report":
        return this.flights.report((p.day as string | null) ?? null);
      case "gear_preflight":
        return this.flights.preflight();
      case "gear_crashes":
        return this.flights.crashList((p.clip as string | null) ?? null, (p.aircraft as string | null) ?? null);
      case "gear_crash_save":
        return this.gearChanged(
          this.flights.crashSave(p as import("../types").CrashSaveParams, (id) => {
            const c = this.lib.clips.find((x) => x.id === id);
            return c ? { date: c.date, aircraft: c.aircraft, duration: c.duration } : null;
          }),
        );
      case "gear_crash_delete":
        return this.gearChanged(this.flights.crashDelete(String(p.id)));
      case "modules":
      case "modules_check":
        return structuredClone(this.modules);
      case "module_install":
        return this.moduleInstall(String(p.name), !!p.confirm);
      case "module_remove":
        return this.moduleChange(String(p.name), () => ({ installed: null, folder: null, update: false }));
      default:
        throw `unknown method "${method}"`;
    }
  }

  // ---------- gear ----------

  /** Answers `v` after telling the UI Gear data changed, as the core's writes do. */
  private gearChanged<T>(v: T): T {
    this.emit("gear-changed");
    return structuredClone(v);
  }

  /** `Core::gear_device_save`: a new device must be plugged in. */
  deviceSave(id: string, name: string | null, aircraft: string | null) {
    let d = this.gear.devices.find((x) => x.id === id);
    if (!d) {
      const c = this.gear.connected.find((x) => x.id === id);
      if (!c) throw `No device ${id} is known or plugged in.`;
      d = { id, kind: c.kind, name: "", aircraft: null, identity: c.identity, last_seen: "2026-10-07T12:00:00Z", last_backup: null };
      this.gear.devices.push(d);
    }
    if (name != null) d.name = name.trim();
    if (aircraft != null) {
      if (aircraft && !(this.settings.values.profiles || []).some((p) => p.name === aircraft)) throw `No aircraft profile named ${aircraft}.`;
      d.aircraft = aircraft || null;
    }
    this.emit("gear-changed");
    return structuredClone(d);
  }

  /** `Core::gear_backup`: the device plugged in that `p` names (or the only one). */
  gearBackup(p: Record<string, unknown>) {
    const c = this.gear.connected.find((x) => (p.device && x.id === p.device) || (p.mount && x.link.kind === "volume" && x.link.mount === p.mount) || (p.port && x.link.kind === "serial" && x.link.port === p.port)) || (this.gear.connected.length === 1 ? this.gear.connected[0] : undefined);
    if (!c?.id) throw "No device: no radio card or FC is plugged in.";
    const r = backups.take(this.gear, c.id, new Date().toISOString());
    this.emit("gear-changed");
    return r;
  }

  /** Sends `device-changed` the way the app's poll does: the events, and what is plugged in
   *  and unmounted now. */
  plug(connected: import("../types").Connected[], unmounted: import("../types").Connected[] = []) {
    const before = new Set(this.gear.connected.map((c) => JSON.stringify(c.link)));
    this.gear.connected = connected;
    const events = connected.filter((c) => !before.has(JSON.stringify(c.link))).map((device) => ({ kind: "connected", device, app_initiated: false }));
    this.emit("device-changed", { events, connected: gear.gearStatus(this.gear, this.settings.values).connected, unmounted });
  }

  /** Plugs the radio in (USB Joystick mode) or pulls it. */
  radioPlug(on: boolean) {
    this.radio.connected = on;
    if (!on) this.radio.frame = null;
    this.emitRadio();
  }

  /** The radio sends a report: raw axes 0..2048 (CH1-8) and button bits. */
  radioFrame(axes: number[], buttons = 0) {
    this.radio.connected = true;
    const seq = (this.radio.frame?.seq ?? 0) + 1;
    this.radio.frame = { seq, buttons, axes, channels: axes.map((a) => 988 + Math.floor(Math.min(2048, a) / 2)) };
    this.emitRadio();
    const v = this.sim.frame(axes, buttons);
    if (v) this.emit("sim-calibration-event", v);
  }

  private emitRadio() {
    if (!this.radio.watching) return;
    this.emit("radio-input", { connected: this.radio.connected, product: this.radio.connected ? "Test Radio Joystick" : null, frame: this.radio.frame });
  }

  // ---------- modules ----------

  ffmpegTools() {
    const ff = this.modules.find((m) => m.name === "ffmpeg");
    const dir = ff?.folder && this.settings.values.ffmpegSource !== "homebrew" ? ff.folder : "/opt/homebrew/bin";
    return { ffmpeg: `${dir}/ffmpeg`, ffprobe: `${dir}/ffprobe` };
  }

  moduleChange(name: string, patch: (m: ModuleStatus) => Partial<ModuleStatus>): ModuleStatus {
    const m = this.modules.find((x) => x.name === name);
    if (!m) throw `unknown module "${name}"`;
    Object.assign(m, patch(m));
    return structuredClone(m);
  }

  moduleInstall(name: string, confirm: boolean): ModuleStatus {
    if (!confirm) throw "Refused: the install needs confirm=true.";
    return this.moduleChange(name, (m) => {
      const pin = m.newest ?? m.pinned;
      return {
        installed: {
          name,
          version: pin.version,
          installed_at: "2026-10-07T12:00:00Z",
          license: pin.license,
          license_url: pin.license_url,
          source: pin.source,
          homepage: pin.homepage,
          assets: pin.assets,
          tools: Object.fromEntries(Object.entries(pin.tools).map(([t, path]) => [t, { path, sha256: "0".repeat(64), signing: "upstream" as const }])),
          size: 130_000_000,
        },
        folder: `${seed.HOME}/Library/Application Support/app.quadcam/modules/${name}/${pin.version}`,
        update: false,
        problem: null,
      };
    });
  }

  // ---------- library ----------

  libraryView(): LibraryView {
    seed.recount(this.lib);
    return structuredClone(this.lib);
  }

  clip(id: string): LibClip {
    const c = this.lib.clips.find((x) => x.id === id);
    if (!c) throw `No clip ${id} in the library.`;
    return c;
  }

  private libraryChanged() {
    seed.recount(this.lib);
    this.emit("library-changed");
  }

  rate(ids: string[], rating: number | null, flag: LibClip["flag"] | null) {
    if (rating != null && rating > 5) throw "A rating is 0 to 5 stars.";
    const out = ids.map((id) => {
      const c = this.clip(id);
      if (rating != null) c.rating = rating;
      if (flag != null) c.flag = flag;
      return structuredClone(c);
    });
    this.libraryChanged();
    return out;
  }

  edit(id: string, e: LibEdit) {
    const c = this.clip(id);
    if (e.note != null) c.note = e.note;
    if (e.keywords != null) c.keywords = e.keywords;
    if (e.author != null) c.author = e.author || null;
    if (e.place != null) {
      const pl = (this.settings.values.places || []).find((x) => x.name.toLowerCase() === e.place!.toLowerCase());
      if (e.place && !pl) throw `No saved place named ${e.place}.`;
      c.place = pl ? pl.name : null;
      c.location = pl ? { lat: pl.lat, lon: pl.lon, name: pl.name } : null;
    }
    if (e.location) {
      c.location = normLocation(e.location);
      c.place = e.location.name || null;
    }
    if (e.profile != null) c.aircraft = e.profile || null;
    if (e.time != null) c.time = e.time || null;
    if (e.date && e.date !== c.date) {
      const old = c.date;
      c.path = c.path.split(old).join(e.date);
      c.file = `${this.lib.root}/${c.path}`;
      c.date = e.date;
    }
    this.libraryChanged();
    return structuredClone(c);
  }

  rename(id: string, name: string) {
    const c = this.clip(id);
    name = name.trim();
    if (!name) throw "A name is required.";
    const dir = dirOf(c.path);
    c.path = `${dir}/${c.date}_${slug(name)}.mp4`;
    c.file = `${this.lib.root}/${c.path}`;
    c.title = name;
    c.name = name;
    this.libraryChanged();
    return structuredClone(c);
  }

  libraryCuts(id: string, cuts: Span[], decision: string | null): CutChange {
    const c = this.clip(id);
    const removed = c.cuts.filter((k) => !cuts.some((n) => sameSpan(n, k)));
    if (removed.length && !decision) return { status: "confirm", files: removed.map((k) => `${this.lib.root}/${k.path}`) };
    c.cuts = c.cuts.filter((k) => cuts.some((n) => sameSpan(n, k)));
    c.pending_cuts = cuts.filter((n) => !c.cuts.some((k) => sameSpan(n, k)));
    const files = removed.map((k) => `${this.lib.root}/${k.path}`);
    this.libraryChanged();
    return { status: "applied", cuts, kept: decision === "keep" ? files : [], trashed: decision === "trash" ? files : [] };
  }

  exportCuts(id: string): LibCut[] {
    const c = this.clip(id);
    const stem = c.path.replace(/\.mp4$/, "");
    const made: LibCut[] = [];
    const total = c.pending_cuts.length;
    c.pending_cuts.forEach((k, i) => {
      this.emit("library-task", { task: "cuts", done: i, total });
      const cut = { path: `${stem}_cut${c.cuts.length + 1}.mp4`, start: k.start, end: k.end, size: Math.round((k.end - k.start) * 46_000) };
      c.cuts.push(cut);
      made.push(cut);
    });
    this.emit("library-task", { task: "cuts", done: total, total });
    c.pending_cuts = [];
    this.libraryChanged();
    return made;
  }

  trashClips(ids: string[]) {
    const moved: Moved[] = [];
    const trashed: string[] = [];
    for (const id of ids) {
      const c = this.clip(id);
      for (const rel of [c.path, ...c.cuts.map((k) => k.path), ...(c.original ? [c.original] : [])]) {
        const from = `${this.lib.root}/${rel}`;
        const to = `${seed.HOME}/.Trash/${base(rel)}`;
        moved.push({ id, from, to });
        trashed.push(from);
      }
      this.trash.set(id, c);
      this.lib.clips = this.lib.clips.filter((x) => x.id !== id);
    }
    this.libraryChanged();
    return { trashed, failed: [], moved };
  }

  untrash(moved: Moved[]) {
    const ids = [...new Set(moved.map((m) => m.id))];
    for (const id of ids) {
      const c = this.trash.get(id);
      if (!c) throw `${id} is no longer in the Trash.`;
      this.trash.delete(id);
      this.lib.clips.push(c);
    }
    this.libraryChanged();
    return ids;
  }

  libraryPhotos(ids: string[], album: string) {
    const added = ids.flatMap((id) => {
      const c = this.clip(id);
      c.in_photos = true;
      return [c.file, ...c.cuts.map((k) => `${this.lib.root}/${k.path}`)];
    });
    this.libraryChanged();
    return { added, failed: [], album: album || null };
  }

  strips() {
    const todo = this.lib.clips.filter((c) => !c.strip && !c.poster && !c.no_picture);
    todo.forEach((c, i) => {
      this.emit("library-task", { task: "thumbnails", done: i, total: todo.length });
      c.strip = seed.stripOf(c);
    });
    if (todo.length) {
      this.emit("library-task", { task: "thumbnails", done: todo.length, total: todo.length });
      this.libraryChanged();
    }
    return todo.length;
  }

  // ---------- settings ----------

  settingsSet(values: Record<string, unknown>) {
    const v = this.settings.values as Record<string, unknown>;
    for (const [k, x] of Object.entries(values)) {
      if (x === null) delete v[k];
      else v[k] = x;
    }
    if (typeof values.outputDir === "string") this.lib.root = values.outputDir;
    this.emit("settings-changed");
    return structuredClone(this.settings);
  }

  placeSearch(query: string): GeoResult[] {
    if (/nowhere/i.test(query)) return [];
    return [
      { name: query, address: `1 ${query} Road, Springfield`, lat: 40.2, lon: -75.2, provider: "apple" },
      { name: `${query} North`, address: `9 ${query} Lane, Springfield`, lat: 40.25, lon: -75.25, provider: "apple" },
    ];
  }

  placeSave(name: string, lat: number, lon: number) {
    const places = [...(this.settings.values.places || [])];
    if (places.some((p) => p.name.toLowerCase() === name.toLowerCase())) throw `A place named ${name} exists already.`;
    places.push({ name, lat, lon });
    this.settingsSet({ places });
    return { place: { name, lat, lon }, from_search: null };
  }

  // ---------- the import session ----------

  private sessionChanged() {
    this.emit("session-changed");
  }

  private need(): Session {
    if (!this.session) throw "No session. Load a card or a folder first.";
    return this.session;
  }

  load(source: string): Session {
    const s = seed.reviewSession(source);
    const n = s.clips.length;
    for (let i = 0; i < n; i++) this.emit("progress", { phase: "stage", index: i, total: n, done: s.clips[i].size, size: s.clips[i].size });
    for (let i = 0; i < n; i++) this.emit("progress", { phase: "analyse", index: i, total: n, done: 0, size: 0 });
    if (source.startsWith("/Volumes/")) {
      const v = this.volumes.find((x) => x.mount === source);
      if (v) {
        s.card_volume = v;
        s.card = { device_identifier: v.info.device_identifier, whole_disk: v.info.parent_whole_disk, volume_uuid: v.info.volume_uuid, volume_name: v.info.volume_name, total_size: v.info.total_size, media_name: v.info.media_name };
        if (v.source === "dji") asDji(s);
      }
    }
    this.session = s;
    this.sessionChanged();
    return structuredClone(s);
  }

  planDates(logDir: string | null, day: string | null): Session {
    const s = this.need();
    s.log_dir = logDir || null;
    s.log_day = day;
    s.log_days = logDir ? ["2026-09-28", "2026-09-27"] : [];
    this.sessionChanged();
    return structuredClone(s);
  }

  plan(id: number): ClipPlan {
    const p = this.need().plans.find((x) => x.id === id);
    if (!p) throw `No clip ${id} in the session.`;
    return p;
  }

  patch(patches: PlanPatch[]): Session {
    const s = this.need();
    for (const x of patches) {
      const head = s.clips.find((c) => c.id === x.id)?.part_of;
      if (head != null) throw `clip ${x.id} is part of clip ${head}, one recording; set joined false on clip ${head} to change it on its own`;
      if (x.joined != null) this.setJoined(x.id, x.joined);
      const p = this.plan(x.id);
      if (x.date != null) {
        p.date = x.date;
        p.source = "edited";
        p.suggested.date = false;
      }
      if (x.time != null) {
        p.time = x.time ? (x.time.length === 5 ? `${x.time}:00` : x.time) : null;
        p.suggested.date = false;
      }
      if (x.name != null) {
        p.name = x.name;
        p.suggested.name = false;
      }
      if (x.note != null) {
        p.note = x.note;
        p.suggested.note = false;
      }
      if (x.skip != null) {
        p.skip = x.skip;
        p.suggested.skip = false;
      }
      if (x.cuts != null) {
        p.cuts = normSpans(x.cuts);
        p.suggested.cuts = false;
      }
      if (x.log_offset_s != null) p.log_offset_s = x.log_offset_s;
      if (x.profile != null) p.meta.profile = x.profile || null;
      if (x.place != null) {
        const pl = (this.settings.values.places || []).find((q) => q.name.toLowerCase() === x.place!.toLowerCase());
        p.meta.location = pl ? { lat: pl.lat, lon: pl.lon, name: pl.name } : null;
      }
      if (x.location) p.meta.location = normLocation(x.location);
      if (x.keywords != null) p.meta.keywords = x.keywords;
      if (x.author != null) p.meta.author = x.author || null;
      if (x.profile != null || x.place != null || x.location || x.keywords != null || x.author != null) p.suggested.meta = false;
      if (!Object.values(p.suggested).some(Boolean)) p.reason = null;
    }
    this.sessionChanged();
    return structuredClone(s);
  }

  /** Joins a split recording into one clip, or keeps its files apart, as `Session::set_joined`. */
  setJoined(id: number, on: boolean) {
    const s = this.need();
    const c = s.clips.find((x) => x.id === id);
    if (!c?.join) throw `clip ${id} is not the first file of a recording the DVR split`;
    if (c.join.on === on) return;
    const own = c.duration;
    c.duration = c.join.swap.duration ?? own;
    c.join.swap.duration = own;
    c.join.on = on;
    for (const x of s.clips) if (c.join.parts.includes(x.id)) x.part_of = on ? id : null;
    if (!on) this.plan(id).cuts = this.plan(id).cuts.filter((k) => k.start + 0.5 <= c.duration).map((k) => ({ ...k, end: Math.min(k.end, c.duration) }));
  }

  sessionCuts(id: number, cuts: Span[], decision: string | null): CutChange {
    const s = this.need();
    const r = [...s.results].reverse().find((x) => x.id === id);
    const done = (r?.cuts || []).filter((k) => k.outcome === "verified");
    const removed = done.filter((k) => !cuts.some((n) => sameSpan(n, k)));
    if (removed.length && !decision) return { status: "confirm", files: removed.map((k) => k.output!) };
    this.plan(id).cuts = cuts;
    this.sessionChanged();
    const files = removed.map((k) => k.output!);
    return { status: "applied", cuts, kept: decision === "keep" ? files : [], trashed: decision === "trash" ? files : [] };
  }

  /** Adds the session's suggestion marks as an agent would (`quadcam_suggest`). */
  agentSuggest(id: number, fields: Partial<Pick<ClipPlan, "name" | "note">>, reason: string) {
    const p = this.plan(id);
    if (fields.name != null) {
      p.name = fields.name;
      p.suggested.name = true;
    }
    if (fields.note != null) {
      p.note = fields.note;
      p.suggested.note = true;
    }
    p.reason = reason;
    this.sessionChanged();
  }

  /** Puts a clip in the library, replacing the one with its id, as `Library::upsert` does.
   * Unsaved cuts survive. */
  upsert(clip: LibClip) {
    const at = this.lib.clips.findIndex((x) => x.id === clip.id);
    if (at < 0) this.lib.clips.push(clip);
    else this.lib.clips[at] = { ...clip, pending_cuts: clip.pending_cuts.length ? clip.pending_cuts : this.lib.clips[at].pending_cuts };
  }

  importClips(opts: { output_dir: string; format: string; keep_clips?: boolean }) {
    const s = this.need();
    const defaultName = (this.settings.values.defaultName as string) || "flight";
    const results: ClipResult[] = [];
    let imported = 0;
    for (const p of s.plans) {
      const c = s.clips.find((x) => x.id === p.id)!;
      if (c.part_of != null) continue;
      if (p.skip || c.status === "empty") {
        const r: ClipResult = { id: p.id, outcome: "skipped", output: null, original: null, size: 0, error: null, encoder: null, meta: null, cuts: [], qt: [] };
        results.push(r);
        this.emit("import-result", r);
        continue;
      }
      this.emit("import-progress", { id: p.id, seconds: c.duration / 2, duration: c.duration });
      this.emit("import-progress", { id: p.id, seconds: c.duration, duration: c.duration });
      const name = slug(p.name || defaultName);
      const dir = `${opts.output_dir}/${p.date.slice(0, 4)}/${p.date}`;
      const output = `${dir}/${p.date}_${name}.${opts.format}`;
      const cuts = p.cuts.map((k, i) => ({ start: k.start, end: k.end, outcome: "verified" as const, output: `${dir}/${p.date}_${name}_cut${i + 1}.${opts.format}`, size: Math.round((k.end - k.start) * 46_000), error: null }));
      const r: ClipResult = { id: p.id, outcome: "verified", output, original: null, size: Math.round(c.duration * 46_000), error: null, encoder: "videotoolbox", meta: null, cuts, qt: [] };
      results.push(r);
      this.emit("import-result", r);
      imported++;
      const loc = p.meta.location;
      this.upsert({
        id: c.key || `k${p.id}`, path: output.slice(opts.output_dir.length + 1), title: p.name, note: p.note, date: p.date, time: p.time ? p.time.slice(0, 5) : null,
        duration: c.duration, size: r.size, rating: 0, flag: "none", place: loc?.name || null, location: loc || null, aircraft: p.meta.profile || (this.settings.values.defaultProfile as string) || null,
        keywords: ["FPV", ...p.meta.keywords], author: p.meta.author, moments: p.moments, keep: c.signal?.keep || [], stats: p.flight, cuts: cuts.map((k) => ({ path: k.output.slice(opts.output_dir.length + 1), start: k.start, end: k.end, size: k.size })),
        pending_cuts: [], in_photos: false, original: null, aliases: [], parts: (c.join?.on ? c.join.parts : []).map((id) => { const x = s.clips.find((y) => y.id === id)!; return { source: x.key, dvr: x.name }; }), dvr: c.name, import: "20261001-120000", cut_of: null, name: p.name || defaultName, file: output, strip: null, poster: null, no_picture: false, last_import: true,
      });
    }
    this.lib.last_import = "20261001-120000";
    s.results = results;
    s.output_dir = opts.output_dir;
    this.libraryChanged();
    this.sessionChanged();
    const summary = { results, imported, skipped: results.length - imported, failed: 0, total_bytes: results.reduce((a, r) => a + r.size, 0), output_dir: opts.output_dir, format_ready: s.kind === "dji" ? { Err: "QuadCam does not format DJI cards after an import. Card prep can erase a removable card once every clip is in the library." } : s.card ? { Ok: null } : { Err: "Clips came from a folder, not a card." } };
    // "Delete clips after import": only the setting turns it on; a run may turn it off.
    const clip_deletion =
      this.settings.values.deleteClipsAfterImport && !opts.keep_clips
        ? s.clips.map((c) => {
            const r = results.find((x) => x.id === (c.part_of ?? c.id));
            const ok = r?.outcome === "verified";
            return { id: c.id, path: c.card_path, state: ok ? ("deleted" as const) : ("kept" as const), reason: ok ? null : "it was skipped" };
          })
        : null;
    return { summary, photos: null, clip_deletion };
  }

  addToPhotos(ids: number[] | null, album: string | null) {
    const s = this.need();
    const ok = s.results.filter((r) => r.outcome === "verified" && (!ids || ids.includes(r.id)));
    s.in_photos = [...new Set([...s.in_photos, ...ok.map((r) => r.id)])];
    this.sessionChanged();
    return { added: ok.flatMap((r) => [r.output!, ...r.cuts.map((k) => k.output!)]), failed: [], album };
  }

  formatPlan(label: string | null) {
    const s = this.need();
    if (s.kind === "dji") throw "Refused: QuadCam does not format DJI cards after an import. Card prep can erase a removable card once every clip is in the library.";
    if (!s.card) throw "Clips came from a folder, not a card.";
    return { disk: "disk9", device: "/dev/disk9", volume_uuid: s.card.volume_uuid || "", volume_name: s.card.volume_name || "", size: s.card.total_size, media_name: s.card.media_name || "", clip_count: s.clips.length, label: label || "DVR", filesystem: "FAT32", warnings: s.card.total_size > 34e9 ? ["Card is larger than 32 GB. Most analog DVRs take cards up to 32 GB; this DVR may not read it."] : [] };
  }

  formatCard(label: string) {
    const plan = this.formatPlan(label);
    const s = this.need();
    s.card = null;
    s.card_volume = null;
    s.warnings = [...s.warnings, "The card was erased and is safe to remove."];
    this.volumes = this.volumes.filter((v) => !v.is_card);
    this.sessionChanged();
    this.emit("volumes-changed");
    return plan;
  }
}

/** Makes a recorded analog session read as a DJI one: the source kind, and dates from the
 * clip clock where the import date stood. */
function asDji(s: Session) {
  s.kind = "dji";
  for (const c of s.clips) c.kind = "dji";
  for (const p of s.plans)
    if (p.source === "import") {
      p.source = "clip";
      p.time = p.time || "18:30:00";
    }
}

/** Forty more clips over ten days, for scrolling and virtualization. */
function manyClips(lib: LibraryView): LibraryView {
  const t = lib.clips[0];
  for (let d = 0; d < 10; d++) {
    const date = `2026-08-${String(10 + d).padStart(2, "0")}`;
    for (let i = 0; i < 4; i++) {
      const name = `flight-${d}-${i}`;
      const path = `2026/${date}/${date}_${name}.mp4`;
      lib.clips.push({ ...structuredClone(t), id: `many-${d}-${i}`, path, file: `${lib.root}/${path}`, title: name, name, date, time: `1${i}:00`, rating: (d + i) % 6, flag: "none", import: "20260810-120000", last_import: false, strip: t.strip });
    }
  }
  return lib;
}

/** One cut per radio-log flight, as `trim::flight_cuts`: 2 s each side (at most half the gap),
 * clamped to the clip. */
function flightCuts(flights: Span[], duration: number): Span[] {
  if (!flights.length) throw "nothing to split; the clip has no radio-log flights (match the logs first)";
  const r = (x: number) => Math.round(x * 10) / 10;
  const ps = [...flights].sort((a, b) => a.start - b.start);
  const out = ps
    .map((p, i) => {
      const before = i ? Math.max(0, (p.start - ps[i - 1].end) / 2) : 2;
      const after = i < ps.length - 1 ? Math.max(0, (ps[i + 1].start - p.end) / 2) : 2;
      return { start: r(Math.max(0, p.start - Math.min(2, before))), end: r(Math.min(duration, p.end + Math.min(2, after))) };
    })
    .filter((k) => k.end - k.start >= 0.5);
  if (!out.length) throw "nothing to split; no radio-log flight falls inside the clip";
  if (out.length === 1 && duration - (out[0].end - out[0].start) < Math.max(10, duration * 0.1)) throw "nothing to split; its one flight covers nearly the whole clip";
  return out;
}

const withSpans = (cuts: Span[], add: Span[]): Span[] => [...cuts.map(({ start, end }) => ({ start, end })), ...add.filter((a) => !cuts.some((c) => sameSpan(c, a)))].sort((a, b) => a.start - b.start);
