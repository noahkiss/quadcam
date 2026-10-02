// A fake core for tests and design work: answers the Tauri commands the UI uses, keeps
// state in memory, and emits the events the real core emits (`library-changed`,
// `session-changed`, `settings-changed`, progress). Shapes follow the recorded fixtures;
// behaviour follows `Core` closely enough for the parity specs.
import type {
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
import { location as normLocation, spans as normSpans } from "../normalize";

/** `Core::dispatch` methods, each also a typed Tauri command of the same name. */
const DISPATCH = new Set([
  "library", "library_rate", "library_edit", "library_rename", "library_cuts", "library_export_cuts", "library_trash", "library_untrash",
  "library_photos", "library_apply_name_format", "library_rebuild", "library_rescan", "library_preview", "library_strips", "card_status",
  "settings", "settings_set", "place_search", "place_save", "session_cuts", "profiles",
]);

export type Scenario = "library" | "empty" | "card" | "review" | "finished-card" | "no-tools" | "many";

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
  trash = new Map<string, LibClip>();
  calls: Call[] = [];
  menuState: unknown = null;
  /** Answers for the folder picker, used in order; then null. */
  dialogAnswers: (string | string[] | null)[] = [];
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
    if (sc === "finished-card") {
      this.session = seed.finishedSession();
      this.session.source = "/Volumes/DVR";
      const v = seed.cardVolume();
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
        return this.importClips(p as { output_dir: string; format: string });
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
          ? { tools: { ffmpeg: "/opt/homebrew/bin/ffmpeg", ffprobe: "/opt/homebrew/bin/ffprobe" }, error: null, install_hint: "brew install ffmpeg", socket: `${seed.HOME}/Library/Application Support/app.quadcam/control.sock` }
          : { tools: null, error: "ffmpeg and ffprobe not found.", install_hint: "brew install ffmpeg", socket: null };
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
        return null;
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
      case "profiles":
        return { profiles: this.settings.values.profiles || [], default_profile: this.settings.values.defaultProfile || null };
      default:
        throw `unknown method "${method}"`;
    }
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

  importClips(opts: { output_dir: string; format: string }) {
    const s = this.need();
    const defaultName = (this.settings.values.defaultName as string) || "flight";
    const results: ClipResult[] = [];
    let imported = 0;
    for (const p of s.plans) {
      const c = s.clips.find((x) => x.id === p.id)!;
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
        pending_cuts: [], in_photos: false, original: null, aliases: [], dvr: c.name, import: "20261001-120000", cut_of: null, name: p.name || defaultName, file: output, strip: null, poster: null, no_picture: false, last_import: true,
      });
    }
    this.lib.last_import = "20261001-120000";
    s.results = results;
    s.output_dir = opts.output_dir;
    this.libraryChanged();
    this.sessionChanged();
    const summary = { results, imported, skipped: results.length - imported, failed: 0, total_bytes: results.reduce((a, r) => a + r.size, 0), output_dir: opts.output_dir, format_ready: s.card ? { Ok: null } : { Err: "Clips came from a folder, not a card." } };
    return { summary, photos: null };
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
    if (!s.card) throw "Clips came from a folder, not a card.";
    return { disk: "disk9", device: "/dev/disk9", volume_uuid: s.card.volume_uuid || "", volume_name: s.card.volume_name || "", size: s.card.total_size, media_name: s.card.media_name || "", clip_count: s.clips.length, label: label || "DVR" };
  }

  formatCard(label: string) {
    const plan = this.formatPlan(label);
    const s = this.need();
    s.card = null;
    s.card_volume = null;
    s.warnings = [...s.warnings, "The card was erased and ejected."];
    this.volumes = this.volumes.filter((v) => !v.is_card);
    this.sessionChanged();
    this.emit("volumes-changed");
    return plan;
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
