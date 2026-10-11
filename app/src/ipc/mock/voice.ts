// The mock core's voice (`core/voice.rs`, `gear/voice/`), written by hand: a few of QuadCam's
// lines, one pack in a made-up index, per-line overrides, a local render, Choose voice (one
// `card_files` change, as the core stages it, on one radio or several) and pack delete.
import type { PreviewLine, PreviewTake, PublishApproveReport, PublishReview, ReviewLine } from "../types";
import type { Catalog, Edit, HeardLine, ImportReport, KeyStatus, LineOverride, ModelInfo, PackDeleteReport, RenderReport, Retake, SampleItem, SampleReport, SetInfo, StagedChange, StudioEstimate, StudioView, VoiceChooseRadiosReport, VoiceEstimate, VoiceCheckReport, VoiceInfo, VoiceLine, VoicePack, VoiceView } from "../types";
import type { MockGear } from "./gear";
import { hint as secretHint } from "./secret";
import * as changes from "./changes";

export const VOICE_CHANGE = "Voice: ";

interface Seed {
  path: string;
  text: string;
  group: string;
  why: string;
}

const LINES: Seed[] = [
  { path: "SOUNDS/en/armed.wav", text: "Armed", group: "callouts", why: "Motors can spin." },
  { path: "SOUNDS/en/disarm.wav", text: "Disarmed", group: "callouts", why: "Motors are off." },
  { path: "SOUNDS/en/lowbat.wav", text: "Battery low", group: "callouts", why: "The pack voltage is under the first warning level." },
  { path: "SOUNDS/en/gpsfix.wav", text: "GPS fix", group: "callouts", why: "The GPS has a fix." },
  { path: "SOUNDS/en/SYSTEM/0000.wav", text: "zero", group: "numbers", why: "The number 0." },
  { path: "SOUNDS/en/SYSTEM/0001.wav", text: "one", group: "numbers", why: "The number 1." },
  { path: "SOUNDS/en/SYSTEM/hello.wav", text: "Hello", group: "system", why: "The radio powered on." },
];

/** `spelling.toml`, as far as the mock needs it. */
const SPELL: Record<string, string> = { GPS: "G.P.S.", DVR: "D.V.R.", VTX: "V.T.X." };
const spoken = (t: string) => t.replace(/\b[A-Za-z]+\b/g, (w) => SPELL[w] ?? w);

export interface MockVoice {
  /** Pack ids on disk. */
  installed: string[];
  /** The index was read. */
  index: boolean;
  overrides: Record<string, Record<string, LineOverride>>;
  chosen: Record<string, string>;
  custom: { path: string; text: string }[];
  /** The provider may charge (the next render waits for confirm). */
  paid: boolean;
  /** An ElevenLabs key is stored (the mock keeps only that fact). */
  /** The stored key; empty for none. */
  keySet: string;
  /** Credits the studio has spent in this run. */
  spent: number;
  /** A line a studio render without a seed leaves out for a re-take. */
  retake: string | null;
  /** The sets rendered per `voice|model`: their lines have a take. */
  rendered: Record<string, string[]>;
  /** Takes an import copied in: lines per voice, model and speed. */
  imported: { voice: string; model: string; speed: number; lines: number }[];
  /** Packs deleted with their raw takes. */
  takesDeleted: string[];
  /** Each rendered pack's lines that need a re-take (its `pack.json` `retakes`). */
  retakes: Record<string, Retake[]>;
  /** What the speech check heard per pack (its `pack.json` `heard`). */
  heard: Record<string, HeardLine[]>;
  /** The speech check is available (macOS 26 or later with the helper). */
  speech: boolean;
  /** A line the speech check hears as "Battery.". */
  misheard: string | null;
}

export const freshVoice = (): MockVoice => ({ installed: [], index: false, overrides: {}, chosen: {}, custom: [], paid: false, keySet: "", spent: 0, retake: null, rendered: {}, imported: [], takesDeleted: [], retakes: {}, heard: {}, speech: true, misheard: null });

type Paid = { confirm?: boolean; digest?: string | null; settings?: { seed?: number | null } | null };

/** The plan's digest, and the core's refusal of a confirm without it or for another plan. */
function digestOf(plan: unknown): string {
  return `mock-${JSON.stringify(plan)}`;
}
function samePlan(p: Paid, digest: string) {
  if (!p.digest) throw "A paid call needs the digest of its estimate: run it without confirm, show the person the cost, then confirm with that digest.";
  if (p.digest !== digest) throw "The plan changed since its estimate (voices, models, sets, seed or characters): estimate again.";
}

const PREVIEW: PreviewLine[] = LINES.slice(0, 3).map((l, i) => ({ path: l.path, text: l.text, file: `previews/en-demo-v1/0${i + 1}.m4a`, sha256: "0".repeat(64) }));
const AVAILABLE: VoicePack = { id: "en-demo-v1", voice: "Demo", lang: "en", provider: "demo", model: "", lines: LINES.length, license: "CC BY 4.0", attribution: "Made up for the mock core", version: "1", installed: false, local: false, bytes: 3_700_000, stale: false, dir: null, firmware: "edgetx", sets: [], made: null, radios: [], retakes: [], preview: PREVIEW, zip_bytes: 1_600_000 };
const LOCAL_ID = "local-say-samantha";
const MADE = "2026-10-01T18:30:00Z";

function packs(v: MockVoice): VoicePack[] {
  const out: VoicePack[] = [];
  const radios = (id: string) => Object.keys(v.chosen).filter((r) => v.chosen[r] === id);
  const mine = (id: string, k: Partial<VoicePack>): VoicePack => ({ ...AVAILABLE, id, license: "Rendered by you; yours to use.", attribution: "", version: "", installed: true, local: true, bytes: 412_000, dir: `/Users/pilot/gear/voices/${id}`, sets: ["quadcam"], made: MADE, radios: radios(id), retakes: v.retakes[id] ?? [], heard: v.heard[id] ?? [], ...k });
  if (v.installed.includes(LOCAL_ID)) out.push(mine(LOCAL_ID, { voice: "Samantha", provider: "say" }));
  for (const id of v.installed.filter((x) => x.startsWith("local-elevenlabs-"))) {
    const [, name = "voice", model = ""] = /^local-elevenlabs-voice-([a-z]+)-(.*)$/.exec(id) ?? [];
    out.push(mine(id, { voice: name.charAt(0).toUpperCase() + name.slice(1), provider: "elevenlabs", model: model.replace(/-/g, "_"), sets: ["quad"], bytes: 2_300_000 }));
  }
  if (v.installed.includes(AVAILABLE.id)) out.push({ ...AVAILABLE, preview: [], zip_bytes: 0, installed: true, dir: `/Users/pilot/gear/voices/${AVAILABLE.id}`, sets: ["quadcam"], made: MADE, radios: radios(AVAILABLE.id), heard: v.heard[AVAILABLE.id] ?? [] });
  else if (v.index) out.push({ ...AVAILABLE });
  return out;
}

function radioOf(g: MockGear, id: string) {
  const d = g.devices.find((x) => x.id === id);
  if (!d) throw `No device "${id}" in QuadCam's list (quadcam-cli gear devices).`;
  if (d.kind !== "radio") throw `"${id}" is not a radio.`;
  return d;
}

const checkPath = (p: string) => {
  if (!/^SOUNDS\/[A-Za-z-]+\/(SYSTEM\/)?[A-Za-z0-9_-]{1,8}\.wav$/.test(p)) throw `"${p}" is not SOUNDS/<language>/<name>.wav or SOUNDS/<language>/SYSTEM/<name>.wav`;
};

/** `gear_voice`. */
export function view(g: MockGear, p: { radio?: string | null; refresh_index?: boolean }): VoiceView {
  const v = g.voice;
  const notes: string[] = [];
  if (p.refresh_index) v.index = true;
  const all = [...LINES, ...v.custom.map((c) => ({ path: c.path, text: c.text, group: "extras", why: "Your own line." }))];
  const ov = p.radio ? (v.overrides[p.radio] ?? {}) : {};
  const ps = packs(v);
  return {
    provider: { provider: "say", base_url: "", model: "", voice: "Samantha", paid: v.paid, key_set: false, ready: true, problem: null },
    lines: all.map<VoiceLine>((l) => ({
      ...l,
      spoken: spoken(l.text),
      custom: !LINES.some((x) => x.path === l.path),
      packs: ps.filter((k) => k.installed).map((k) => k.id),
      override: ov[l.path] ?? null,
    })),
    packs: ps,
    chosen: p.radio ? (v.chosen[p.radio] ?? null) : null,
    index_source: "https://example.invalid/voices.json",
    notes,
    speech: v.speech
      ? { name: "asr", available: true, min_os: "26.0", reason: null, locale: "en-US", assets: "installed" }
      : { name: "asr", available: false, min_os: "26.0", reason: "The speech assets for en-US are not installed: run quadcam-intel install --locale en-US.", locale: "en-US", assets: "supported" },
  };
}

const SPEECH_REASON = (heard: string) => `The speech check heard ${JSON.stringify(heard)}.`;

/** The speech check of a pack's lines (`core/voice_check.rs`): the mock hears each line's
 *  text, except `misheard`, which it hears as "Battery.". A studio pack holds a line heard
 *  wrong out as a re-take. */
function hear(v: MockVoice, id: string, only: string[] = []): VoiceCheckReport {
  const lines: HeardLine[] = [];
  const studio = id.startsWith("local-elevenlabs-");
  let held = 0;
  for (const l of LINES) {
    if (only.length && !only.includes(l.path)) continue;
    if ((v.retakes[id] ?? []).some((r) => r.path === l.path)) continue;
    const heard = l.path === v.misheard ? "Battery." : `${l.text}.`;
    const ok = heard !== "Battery.";
    const before = (v.heard[id] ?? []).find((h) => h.path === l.path);
    const accepted = !ok && !!before?.accepted && before.heard === heard;
    lines.push({ path: l.path, text: l.text, heard, ok, accepted, confidence: 0.9 });
    if (!ok && !accepted && studio) {
      (v.retakes[id] ??= []).push({ path: l.path, text: l.text, reason: SPEECH_REASON(heard) });
      held++;
    }
  }
  const keep = (v.heard[id] ?? []).filter((h) => !lines.some((x) => x.path === h.path));
  v.heard[id] = [...keep, ...lines].sort((a, b) => a.path.localeCompare(b.path));
  const mismatches = lines.filter((h) => !h.ok && !h.accepted).length;
  const notes = held ? [`${held} lines the check heard wrong are out of the pack as re-takes: render again with another seed to redo them (priced first), or accept what was heard.`] : [];
  return { pack: id, available: true, reason: null, lines, mismatches, held, notes };
}

/** `gear_voice_check`. */
export function check(g: MockGear, p: { pack: string; lines?: string[]; accept?: string[]; install?: boolean }): VoiceCheckReport {
  const v = g.voice;
  if (!v.installed.includes(p.pack)) throw `Pack "${p.pack}" is not on this Mac.`;
  const notes: string[] = [];
  if (p.install) {
    v.speech = true;
    notes.push("The speech assets for en-US are installed.");
  }
  if (!v.speech) return { pack: p.pack, available: false, reason: view(g, {}).speech.reason, lines: [], mismatches: 0, held: 0, notes };
  if (p.accept?.length) {
    const lines: HeardLine[] = [];
    for (const path of p.accept) {
      const h = (v.heard[p.pack] ?? []).find((x) => x.path === path);
      if (!h) throw `The speech check has not heard "${path}" in the pack ${p.pack}.`;
      h.accepted = !h.ok;
      lines.push({ ...h });
      v.retakes[p.pack] = (v.retakes[p.pack] ?? []).filter((r) => !(r.path === path && r.reason.startsWith("The speech check heard")));
    }
    return { pack: p.pack, available: true, reason: null, lines, mismatches: 0, held: 0, notes };
  }
  const r = hear(v, p.pack, p.lines ?? []);
  return { ...r, notes: [...notes, ...r.notes] };
}

/** `gear_voice_pack_install`. */
export function install(g: MockGear, p: { pack: string }): VoicePack {
  if (p.pack !== AVAILABLE.id) throw `The index lists no pack "${p.pack}": ${AVAILABLE.id}`;
  g.voice.index = true;
  if (!g.voice.installed.includes(p.pack)) g.voice.installed.push(p.pack);
  return packs(g.voice).find((k) => k.id === p.pack)!;
}

/** `gear_voice_render`. */
export function render(g: MockGear, p: { voice?: string; lines?: string[]; dry_run?: boolean; confirm?: boolean; sets?: string[]; model?: string }): RenderReport {
  const v = g.voice;
  if (p.sets?.length) return studioRender(g, p as never);
  const n = p.lines?.length ? p.lines.length : LINES.length + v.custom.length;
  const chars = LINES.reduce((c, l) => c + spoken(l.text).length, 0);
  const have = v.installed.includes(LOCAL_ID);
  const plan = { lines: n, cached: have ? n : 0, to_render: have ? 0 : n, chars: have ? 0 : chars };
  const base = { pack: LOCAL_ID, provider: "say", voice: p.voice || "Samantha", plan, paid: v.paid, dry_run: !!p.dry_run, notes: [] as string[] };
  if (v.paid && plan.to_render > 0 && !p.confirm && !p.dry_run) return { ...base, rendered: 0, from_cache: 0, needs_confirm: true };
  if (p.dry_run) return { ...base, rendered: 0, from_cache: plan.cached, needs_confirm: false };
  if (!have) v.installed.push(LOCAL_ID);
  return { ...base, rendered: plan.to_render, from_cache: plan.cached, needs_confirm: false };
}

/** `gear_voice_edit`. */
export function edit(g: MockGear, p: { radio: string; line: string; text?: string | null; pack?: string | null; confirm?: boolean; dry_run?: boolean; digest?: string | null }): VoiceLine {
  radioOf(g, p.radio);
  checkPath(p.line);
  const v = g.voice;
  if (p.text && p.pack) throw "Pass a pack or a text for a line, not both.";
  const known = LINES.some((l) => l.path === p.line) || v.custom.some((c) => c.path === p.line);
  if (!known && !p.text) throw `QuadCam has no line ${p.line}: give the text to add it as your own line.`;
  const per = (v.overrides[p.radio] ??= {});
  if (p.pack) {
    if (!v.installed.includes(p.pack)) throw `Pack "${p.pack}" is not installed.`;
    per[p.line] = { kind: "pack", pack: p.pack, text: null };
  } else if (p.text) {
    if (!p.text.trim()) throw "The text is empty.";
    const cost = { chars: spoken(p.text).length, paid: v.paid, provider: "openai", digest: `edit-${p.radio}-${p.line}-${p.text}` };
    if (p.dry_run) {
      const line = view(g, { radio: p.radio }).lines.find((l) => l.path === p.line) ?? { path: p.line, text: p.text, spoken: spoken(p.text), group: "extras", why: "Your own line.", custom: true, packs: [], override: null };
      return { ...line, cost };
    }
    if (v.paid && !p.confirm) throw `This render sends ${cost.chars} characters to a provider that may charge for them. Run it with dry_run, show the person the characters, then confirm with that digest.`;
    if (v.paid && p.digest !== cost.digest) throw p.digest ? "The plan changed since its estimate (voices, models, sets, seed or characters): estimate again." : "A paid call needs the digest of its estimate: run it without confirm, show the person the cost, then confirm with that digest.";
    if (!known) v.custom.push({ path: p.line, text: p.text });
    per[p.line] = { kind: "text", pack: null, text: p.text };
  } else delete per[p.line];
  return view(g, { radio: p.radio }).lines.find((l) => l.path === p.line)!;
}

/** `gear_voice_choose`: one open voice change per radio. */
export function choose(g: MockGear, p: { radio: string; pack: string; keep_overrides?: boolean }, editor: "user" | "agent"): StagedChange {
  radioOf(g, p.radio);
  const v = g.voice;
  const pk = packs(v).find((k) => k.id === p.pack && k.installed);
  if (!pk) throw `Pack "${p.pack}" is not installed: install it first.`;
  const keep = p.keep_overrides !== false;
  const ov = keep ? (v.overrides[p.radio] ?? {}) : {};
  if (!keep) delete v.overrides[p.radio];
  v.chosen[p.radio] = p.pack;
  const paths = [...new Set([...LINES.map((l) => l.path), ...Object.keys(ov)])].sort();
  const put = paths.map((path) => ({ path, xxh64: "0000000000000000", size: 1000 }));
  const edits: Edit[] = [{ kind: "card_files", put, delete: [] }];
  const title = `${VOICE_CHANGE}${pk.voice}`;
  const open = changes.list(g, p.radio, false).find((c) => c.title.startsWith(VOICE_CHANGE) && ["draft", "ready"].includes(c.status) && c.edits.every((e) => e.kind === "card_files"));
  if (open) return changes.update(g, { id: open.id, edits, title });
  return changes.stage(g, p.radio, edits, title, editor);
}

/** `gear_voice_choose_radios`: every radio checked first, then one change each. */
export function chooseRadios(g: MockGear, p: { pack: string; radios?: string[]; all?: boolean; keep_overrides?: boolean }, editor: "user" | "agent"): VoiceChooseRadiosReport {
  const radios = [...(p.radios ?? [])];
  if (p.all) for (const d of g.devices) if (d.kind === "radio" && (!d.identity?.firmware || d.identity.firmware.toLowerCase() === "edgetx") && !radios.includes(d.id)) radios.push(d.id);
  if (radios.length === 0) throw "Name at least one radio, or ask for all radios.";
  for (const r of radios) {
    const d = radioOf(g, r);
    const f = d.identity?.firmware;
    if (f && f.toLowerCase() !== "edgetx") throw `${d.name || r} runs ${f}: voice packs are for EdgeTX radios.`;
  }
  if (!g.voice.installed.includes(p.pack)) throw `Pack "${p.pack}" is not installed: install it first.`;
  return { pack: p.pack, staged: radios.map((radio) => choose(g, { radio, pack: p.pack, keep_overrides: p.keep_overrides }, editor)) };
}

/** `gear_voice_pack_delete`. */
export function remove(g: MockGear, p: { pack: string; takes?: boolean }): PackDeleteReport {
  const k = packs(g.voice).find((x) => x.id === p.pack && x.installed);
  if (!k) throw `Pack "${p.pack}" is not installed.`;
  g.voice.installed = g.voice.installed.filter((x) => x !== p.pack);
  if (p.takes) g.voice.takesDeleted.push(p.pack);
  // A choice of a pack that is gone names nothing, and nor does an override that takes its line.
  for (const r of k.radios ?? []) delete g.voice.chosen[r];
  const overrides: { radio: string; line: string }[] = [];
  for (const [radio, per] of Object.entries(g.voice.overrides))
    for (const [line, o] of Object.entries(per))
      if (o.kind === "pack" && o.pack === p.pack) {
        delete per[line];
        overrides.push({ radio, line });
      }
  return { pack: p.pack, bytes: k.bytes, takes: p.takes ? k.lines : 0, radios: k.radios ?? [], overrides };
}

/** `gear_voice_preview`: a path under the cache, as the core's copy would be. */
export function preview(g: MockGear, p: { line: string; pack?: string | null; radio?: string | null }): string {
  checkPath(p.line);
  const stem = p.line.replace(/^SOUNDS\//, "").replace(/\.wav$/, "").replace(/[^A-Za-z0-9]+/g, "-");
  if (p.pack) {
    if (!g.voice.installed.includes(p.pack)) throw `Pack "${p.pack}" is not installed.`;
    return `/Users/pilot/Library/Caches/app.quadcam/voice/preview/${p.pack}-${stem}.wav`;
  }
  if (p.radio && g.voice.overrides[p.radio]?.[p.line]) return `/Users/pilot/Library/Caches/app.quadcam/voice/preview/own-${stem}.wav`;
  throw `${p.line} has no override on this radio.`;
}

// ----- the voice studio (`core/voice_studio.rs`) -----

const VOICES: VoiceInfo[] = ["Callum", "Daniel", "Brian", "Adam", "Matilda", "Alice", "Sarah", "Lily"].map((name, i) => ({ id: `voice-${name.toLowerCase()}`, name, category: "premade", labels: i < 4 ? "male" : "female", preview_url: null }));
const MODELS: ModelInfo[] = [
  { id: "eleven_v4", name: "Eleven v4", cost_per_char: 1, usd_per_1k: 0.08, promo_until: null, max_chars: 5000 },
  { id: "eleven_v4_turbo", name: "Eleven v4 Turbo", cost_per_char: 0.5, usd_per_1k: 0.04, promo_until: null, max_chars: 5000 },
  { id: "eleven_turbo_v2_5", name: "Turbo v2.5", cost_per_char: 0.5, usd_per_1k: 0.04, promo_until: null, max_chars: 40000 },
];
const QUAD = ["quad"];
const SETS: SetInfo[] = [
  { id: "quad", title: "FPV quad", about: "What a quad's radio says: its own prompts (power on, alarms, timers), numbers and units for telemetry, and quad callouts such as armed, battery low, signal low, failsafe, turtle mode and the flight modes.", lines: 168, kinds: QUAD },
  { id: "extras", title: "FPV extras", about: "More quad callouts for switches and special functions: arming checks, Betaflight modes, VTX, DVR and recording, rate and PID profiles, TX power, battery percent, link quality, race laps, the finder. Each plays only when a model's special function names it.", lines: 68, kinds: QUAD },
  { id: "easter", title: "Easter eggs", about: 'Short fun lines, such as "Engines hot. Send it!". Each plays only when a model\'s special function names it.', lines: 12, kinds: [] },
  { id: "scripts", title: "Telemetry scripts", about: "The prompts the INAV and ArduPilot (Yaapu) telemetry scripts play on the radio. FPV quad already holds the Betaflight script's prompts.", lines: 90, kinds: [] },
  { id: "edgetx", title: "Full EdgeTX English", about: "Every prompt of the EdgeTX English pack: the radio's prompts, numbers and units, the model prompts for quads, helicopters, planes, gliders and cars, and the telemetry-script prompts.", lines: 311, kinds: ["quad", "heli", "plane", "glider", "car"] },
  { id: "heli", title: "Helicopter", about: "The radio's prompts, numbers and units, and helicopter callouts: throttle hold, idle up, governor, rescue, autorotation, head speed.", lines: 164, kinds: ["heli"] },
  { id: "plane", title: "Plane", about: "The radio's prompts, numbers and units, and plane callouts: stall, airspeed, landing gear, flaps, lights, fuel.", lines: 169, kinds: ["plane"] },
  { id: "glider", title: "Glider", about: "The radio's prompts, numbers and units, and glider callouts: thermal mode, lift and sink, crow, launch, ballast.", lines: 164, kinds: ["glider"] },
  { id: "car", title: "Car", about: "The radio's prompts, numbers and units, and car callouts: drive modes, steering, gears, drift.", lines: 160, kinds: ["car"] },
  { id: "sample", title: "Sample", about: "A dozen hard lines for comparing voices: a bare number, short words, warnings.", lines: 12, kinds: [] },
  { id: "quadcam", title: "QuadCam default", about: "The lines of QuadCam's ready-made voice packs: arming, battery, signal, flight modes, timers and numbers.", lines: 45, kinds: [] },
  { id: "custom", title: "Your lines", about: "Lines you added on a radio's Voice segment.", lines: 0, kinds: [] },
];
/** The radio's prompts, numbers and units: every aircraft set and the full set hold them. */
const SHARED = 140;
const holdsShared = (id: string) => ["quad", "heli", "plane", "glider", "car", "edgetx"].includes(id);
const CREDITS = 98_500;
const CARRIER = "The word is .".length;

const keyStatus = (g: MockGear): KeyStatus => (g.voice.keySet ? { set: true, ...secretHint(g.voice.keySet), source: "keychain", problem: null } : { set: false, hint: "", prefix: "", suffix: "", length: 0, source: "", problem: null });

/** `gear_voice_key`. */
export function key(g: MockGear, p: { action?: string; key?: string | null }): KeyStatus {
  if (p.action === "set") {
    const k = p.key?.trim() ?? "";
    if (!k) throw "The key is empty.";
    if (/[\s\p{Cc}]/u.test(k)) throw "A key has no spaces or line breaks: paste only the key.";
    g.voice.keySet = k;
  } else if (p.action === "delete") g.voice.keySet = "";
  return keyStatus(g);
}

/** Lines of a set with a take for this voice and model: all of a rendered set, and the shared
 * radio lines once any set that holds them is rendered. */
function renderedOf(g: MockGear, id: string, voice: string, model: string): number {
  const done = g.voice.rendered[`${voice}|${model}`] ?? [];
  const set = SETS.find((s) => s.id === id);
  if (!set) return 0;
  if (done.includes(id)) return set.lines;
  return holdsShared(id) && done.some(holdsShared) ? Math.min(SHARED, set.lines) : 0;
}

/** `gear_voice_sets`. */
export function sets(g: MockGear, p?: { voice?: string; model?: string } | null): StudioView {
  const custom = g.voice.custom.length;
  const asked = !!(p?.voice && p?.model);
  return {
    key: keyStatus(g),
    sets: SETS.map((s) => {
      const x = s.id === "custom" ? { ...s, lines: custom } : s;
      return { ...x, rendered: asked ? renderedOf(g, x.id, p!.voice!, p!.model!) : null };
    }),
    batch: { carrier: "The word is {line}.", tone_carriers: {}, max_lines: 30, snap_ms: 40 },
    aircraft: ["quad"],
    picks: custom ? ["quad", "extras", "custom"] : ["quad", "extras"],
    rendered: Object.entries(g.voice.rendered)
      .map(([k, ids]) => {
        const [voice, model] = k.split("|");
        const lines = SETS.filter((x) => ids.includes(x.id)).reduce((n, x) => n + x.lines, 0);
        return { voice, model, speed: 1, lines };
      })
      .concat(g.voice.imported)
      .sort((a, b) => b.lines - a.lines),
  };
}

const needKey = (g: MockGear) => {
  if (!g.voice.keySet) throw "No ElevenLabs key is stored: run `quadcam-cli gear voice key set`, or paste it in the Voice studio.";
};

/** `gear_voice_import_takes`: any folder holds 161 takes of Callum on turbo v2.5 at 1.2. */
export function importTakes(g: MockGear, p: { folder: string; dry_run?: boolean }): ImportReport {
  if (!p.folder) throw "Name the folder of takes.";
  const take = { voice: "voice-callum", model: "eleven_turbo_v2_5", speed: 1.2, lines: 161 };
  const already = g.voice.imported.length ? 161 : 0;
  if (!p.dry_run && !already) g.voice.imported.push(take);
  return { found: 162, imported: 161 - already, already, skipped: ["bad/x.json: not a take's JSON (text, voice, model)"], counts: [take], dry_run: !!p.dry_run };
}

/** `gear_voice_catalog`. */
export function catalog(g: MockGear, p: { voices?: boolean; models?: boolean; credits?: boolean }): Catalog {
  needKey(g);
  const all = !(p.voices || p.models || p.credits);
  return {
    voices: all || p.voices ? VOICES : [],
    models: all || p.models ? MODELS : [],
    credits: all || p.credits ? { used: 100_000 - CREDITS + g.voice.spent, limit: 100_000, remaining: CREDITS - g.voice.spent, tier: "creator", resets_at: 0 } : null,
  };
}

function price(g: MockGear, setIds: string[], model: string, voice = ""): VoiceEstimate {
  const m = MODELS.find((x) => x.id === model);
  if (!m) throw `The account lists no model "${model}".`;
  const picked = SETS.filter((s) => setIds.includes(s.id));
  const lines = picked.reduce((n, s) => n + s.lines - renderedOf(g, s.id, voice, model), 0);
  const chars = lines * (CARRIER + 10);
  const credits = Math.ceil(chars * (m.cost_per_char ?? 1));
  const remaining = CREDITS - g.voice.spent;
  const usd = (chars * (m.usd_per_1k ?? 0)) / 1000;
  return { batches: Math.ceil(lines / 30) + 1, cached_batches: 0, lines, chars, cost_per_char: m.cost_per_char, credits_basis: "estimated", usd_per_1k: m.usd_per_1k ?? 0, promo_until: null, credits, usd, remaining, affordable: credits <= remaining };
}

function voiceOf(want: string): VoiceInfo {
  const v = VOICES.find((x) => x.id === want || x.name.toLowerCase() === want.toLowerCase());
  if (!v) throw `The account has no voice "${want}".`;
  return v;
}

/** `gear_voice_estimate`. */
export function estimate(g: MockGear, p: { sets: string[]; voice: string; model: string }): StudioEstimate {
  needKey(g);
  const v = voiceOf(p.voice);
  const e = price(g, p.sets, p.model, v.id);
  const lines = SETS.filter((s) => p.sets.includes(s.id)).reduce((n, s) => n + s.lines, 0);
  return { voice: v.id, voice_name: v.name, model: p.model, lines, rendered: lines - e.lines, retakes: 0, to_render: e.lines, estimate: e };
}

/** `gear_voice_sample`: three lines per voice and model. */
export function sample(g: MockGear, p: { voices: string[]; models: string[]; dry_run?: boolean } & Paid): SampleReport {
  needKey(g);
  const sum = { batches: 0, cached_batches: 0, lines: 0, chars: 0, cost_per_char: 0, credits_basis: "estimated", usd_per_1k: 0, promo_until: null, credits: 0, usd: 0, remaining: CREDITS - g.voice.spent, affordable: true };
  const combos = p.voices.flatMap((v) => p.models.map((m) => ({ v: voiceOf(v), m })));
  for (const c of combos) {
    const e = price(g, ["sample"], c.m);
    sum.batches += e.batches;
    sum.lines += e.lines;
    sum.chars += e.chars;
    sum.credits += e.credits;
    sum.usd += e.usd ?? 0;
  }
  sum.affordable = sum.credits <= sum.remaining;
  const digest = digestOf({ voices: p.voices, models: p.models, seed: p.settings?.seed ?? 0, chars: sum.chars });
  const base = { estimate: sum, combos: combos.length, dry_run: !!p.dry_run, warnings: [] as string[], digest };
  if (p.dry_run) return { ...base, items: [], needs_confirm: false };
  if (!sum.affordable) throw `this render needs ${sum.credits} credits but the account has ${sum.remaining}`;
  if (!p.confirm) return { ...base, items: [], needs_confirm: true };
  samePlan(p, digest);
  g.voice.spent += sum.credits;
  const items = combos.flatMap<SampleItem>((c) =>
    [["SOUNDS/en/SYSTEM/0006.wav", "Six"], ["SOUNDS/en/armed.wav", "Armed"], ["SOUNDS/en/turtle.wav", "Turtle mode"]].map(([line, text]) => ({
      voice: c.v.id,
      voice_name: c.v.name,
      model: c.m,
      line,
      text,
      file: `/Users/pilot/Library/Caches/app.quadcam/voice/samples/${c.v.name.toLowerCase()}-${c.m}/${line.split("/").pop()}`,
      ms: 520,
      heard: g.voice.speech ? (line === g.voice.misheard ? "Battery." : `${text}.`) : null,
      heard_ok: g.voice.speech ? line !== g.voice.misheard : null,
    })),
  );
  return { ...base, items, needs_confirm: false };
}

/** `gear_voice_render` with sets: a batched render into a local pack. */
function studioRender(g: MockGear, p: { voice?: string; sets: string[]; model?: string; dry_run?: boolean } & Paid): RenderReport {
  needKey(g);
  const voice = voiceOf(p.voice ?? "");
  const e = price(g, p.sets, p.model ?? "", voice.id);
  // The core names the pack by a short hash of the voice id; the mock uses the id itself.
  const id = `local-elevenlabs-${voice.id}-${(p.model ?? "").replace(/_/g, "-")}`;
  const seed = p.settings?.seed ?? 0;
  const digest = digestOf({ voice: voice.id, model: p.model, sets: p.sets, seed, chars: e.chars });
  const base = { pack: id, provider: "elevenlabs", voice: voice.name, plan: { lines: e.lines, cached: 0, to_render: e.lines, chars: e.chars }, paid: true, dry_run: !!p.dry_run, notes: [] as string[], estimate: e, warnings: [] as string[], digest, retakes: [] };
  if (p.dry_run) return { ...base, rendered: 0, from_cache: 0, needs_confirm: false };
  if (!e.affordable) throw `this render needs ${e.credits} credits but the account has ${e.remaining}`;
  if (!p.confirm) return { ...base, rendered: 0, from_cache: 0, needs_confirm: true };
  samePlan(p, digest);
  g.voice.spent += e.credits;
  if (!g.voice.installed.includes(id)) g.voice.installed.push(id);
  const k = `${voice.id}|${p.model ?? ""}`;
  g.voice.rendered[k] = [...new Set([...(g.voice.rendered[k] ?? []), ...p.sets])];
  const flagged = LINES.find((l) => l.path === g.voice.retake);
  // The speech check hears the lines rendered into the pack now.
  const heard = () => (g.voice.speech ? hear(g.voice, id).lines : []);
  if (flagged && seed === 0) {
    g.voice.retakes[id] = [{ path: flagged.path, text: flagged.text, reason: "the cut is silent" }];
    const lines = heard();
    return { ...base, rendered: e.batches, from_cache: 0, needs_confirm: false, warnings: [`"${flagged.text}": the cut is silent`], retakes: g.voice.retakes[id], heard: lines };
  }
  // A re-take with a seed redoes only the batches that held a flagged line.
  const redo = (g.voice.retakes[id] ?? []).length;
  delete g.voice.retakes[id];
  if (seed) g.voice.misheard = null;
  const lines = heard();
  const retakes = g.voice.retakes[id] ?? [];
  return { ...base, rendered: seed ? Math.max(1, redo) : e.batches, from_cache: 0, needs_confirm: false, retakes, heard: lines };
}

/** `gear_voice_pack_preview`: the index pack's preview lines, "fetched" into the cache. */
export function packPreview(g: MockGear, p: { pack: string }): PreviewTake[] {
  if (!g.voice.index || p.pack !== AVAILABLE.id) throw `The index lists no pack "${p.pack}".`;
  return PREVIEW.map((l, i) => ({ path: l.path, text: l.text, file: `/mock/cache/voice/index-preview/en-demo-v1-1/0${i + 1}.m4a` }));
}

/** The bundles `gear_voice_publish` prepared, by digest. */
const bundles = new Map<string, PublishReview>();

/** `gear_voice_publish` (`core/voice_publish.rs`): the speech check's gate, then the review. */
export function publish(g: MockGear, p: { pack: string; id?: string | null; version?: string | null; voice_name?: string | null; license?: string | null; attribution?: string | null }): PublishReview {
  const v = g.voice;
  const k = packs(v).find((x) => x.id === p.pack && x.installed);
  if (!k) throw `Pack "${p.pack}" is not on this Mac.`;
  if (k.provider === "say") throw "This pack was made with macOS say: Apple's voices are licensed for use on the Mac, so it cannot be published.";
  const heard = v.heard[p.pack] ?? [];
  const files = LINES.filter((l) => !(v.retakes[p.pack] ?? []).some((r) => r.path === l.path)).map((l) => l.path);
  const unheard = files.filter((f) => !heard.some((h) => h.path === f));
  if (unheard.length) throw `The speech check has not heard ${unheard.length} of the pack's ${files.length} lines: run Check speech (quadcam-cli gear voice check ${p.pack}) first.`;
  const wrong = heard.filter((h) => files.includes(h.path) && !h.ok && !h.accepted);
  if (wrong.length) throw `${wrong.length} lines the speech check heard wrong are not accepted (${wrong.map((h) => h.path).join(", ")}): render them again, or listen and accept what was heard.`;
  const voice = p.voice_name?.trim() || k.voice;
  const id = p.id?.trim() || `en-${voice.toLowerCase()}`;
  const version = p.version?.trim() || "1";
  const license = p.license?.trim() || "ElevenLabs output, paid plan: commercial use allowed";
  const attribution = p.attribution?.trim() || `Voice: ${voice} (ElevenLabs)`;
  const preview: PreviewLine[] = files.slice(0, 10).map((f, i) => ({ path: f, text: LINES.find((l) => l.path === f)!.text, file: `previews/${id}-${version}/${String(i + 1).padStart(2, "0")}.m4a`, sha256: "0".repeat(64) }));
  const digest = `mock-${id}-${version}`;
  const dir = `/mock/cache/voice/publish/${id}-v${version}`;
  const lines: ReviewLine[] = files
    .map((f) => {
      const h = heard.find((x) => x.path === f)!;
      return { path: f, text: h.text, heard: h.heard, ok: h.ok, accepted: !!h.accepted, file: `${dir}/stage/${f}`, spot: true, preview: preview.some((x) => x.path === f) };
    })
    .sort((a, b) => Number(b.accepted) - Number(a.accepted));
  const review: PublishReview = {
    pack: p.pack,
    repo: "noahkiss/quadcam-voices",
    entry: { id, voice, lang: "en", provider: k.provider, model: k.model, settings: { speed: 1, tempo: 1, trim_db: -55, lead_ms: 20, tail_ms: 150, fade_in_ms: 5, fade_out_ms: 30, seed: 0 }, lines: files.length, bytes: 410_000, sha256: "0".repeat(64), license, attribution, lines_csv_sha: "csv", version, file: `voice-${id}-${version}.zip`, firmware: "edgetx", url: `https://github.com/noahkiss/quadcam-voices/releases/download/${id}-v${version}/voice-${id}-${version}.zip`, zip_bytes: 180_000, date: "2026-10-11", preview },
    dir,
    zip: `${dir}/voice-${id}-${version}.zip`,
    lines,
    spot_check: files,
    accepted: lines.filter((l) => l.accepted).length,
    left_out: [],
    digest,
    gh: { ready: true, reason: null },
    commands: [`gh release create ${id}-v${version} ${dir}/voice-${id}-${version}.zip -R noahkiss/quadcam-voices`],
    notes: [],
  };
  bundles.set(digest, review);
  return review;
}

/** `gear_voice_publish_approve`, the review's own Approve click. */
export function approve(p: { digest: string; id?: string | null }): PublishApproveReport {
  const r = bundles.get(p.digest);
  if (!r) throw `No prepared pack has the digest ${p.digest}: prepare it again.`;
  const { id, version } = r.entry;
  if (p.id && p.id !== id) throw `The digest is for ${id}, not "${p.id}": nothing was published.`;
  const published = { release_url: `https://github.com/noahkiss/quadcam-voices/releases/tag/${id}-v${version}`, pr_url: "https://github.com/noahkiss/quadcam-voices/pull/1", branch: `pack/${id}-v${version}` };
  return { id, version, published, commands: [], notes: [`Released ${published.release_url} and opened ${published.pr_url}. Merging the pull request publishes the pack.`] };
}
