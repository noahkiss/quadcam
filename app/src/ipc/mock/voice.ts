// The mock core's voice (`core/voice.rs`, `gear/voice/`), written by hand: a few of QuadCam's
// lines, one pack in a made-up index, per-line overrides, a local render, Choose voice (one
// `card_files` change, as the core stages it, on one radio or several) and pack delete.
import type { Catalog, Edit, KeyStatus, LineOverride, ModelInfo, PackDeleteReport, RenderReport, SampleItem, SampleReport, SetInfo, StagedChange, StudioEstimate, StudioView, VoiceChooseRadiosReport, VoiceEstimate, VoiceInfo, VoiceLine, VoicePack, VoiceView } from "../types";
import type { MockGear } from "./gear";
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
  keySet: boolean;
  /** Credits the studio has spent in this run. */
  spent: number;
  /** Packs deleted with their raw takes. */
  takesDeleted: string[];
}

export const freshVoice = (): MockVoice => ({ installed: [], index: false, overrides: {}, chosen: {}, custom: [], paid: false, keySet: false, spent: 0, takesDeleted: [] });

const AVAILABLE: VoicePack = { id: "en-demo-v1", voice: "Demo", lang: "en", provider: "demo", model: "", lines: LINES.length, license: "CC BY 4.0", attribution: "Made up for the mock core", version: "1", installed: false, local: false, bytes: 3_700_000, stale: false, dir: null, firmware: "edgetx", sets: [], made: null, radios: [] };
const LOCAL_ID = "local-say-samantha";
const MADE = "2026-10-01T18:30:00Z";

function packs(v: MockVoice): VoicePack[] {
  const out: VoicePack[] = [];
  const radios = (id: string) => Object.keys(v.chosen).filter((r) => v.chosen[r] === id);
  const mine = (id: string, k: Partial<VoicePack>): VoicePack => ({ ...AVAILABLE, id, license: "Rendered by you; yours to use.", attribution: "", version: "", installed: true, local: true, bytes: 412_000, dir: `/Users/pilot/gear/voices/${id}`, sets: ["quadcam"], made: MADE, radios: radios(id), ...k });
  if (v.installed.includes(LOCAL_ID)) out.push(mine(LOCAL_ID, { voice: "Samantha", provider: "say" }));
  for (const id of v.installed.filter((x) => x.startsWith("local-elevenlabs-"))) {
    const name = id.split("-")[2] ?? "voice";
    out.push(mine(id, { voice: name.charAt(0).toUpperCase() + name.slice(1), provider: "elevenlabs", model: id.split("-").slice(3).join("_"), sets: ["quad"], bytes: 2_300_000 }));
  }
  if (v.installed.includes(AVAILABLE.id)) out.push({ ...AVAILABLE, installed: true, dir: `/Users/pilot/gear/voices/${AVAILABLE.id}`, sets: ["quadcam"], made: MADE, radios: radios(AVAILABLE.id) });
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
  };
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
export function edit(g: MockGear, p: { radio: string; line: string; text?: string | null; pack?: string | null; confirm?: boolean }): VoiceLine {
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
    if (v.paid && !p.confirm) throw `This render sends ${spoken(p.text).length} characters to a provider that may charge for them. Pass confirm to go ahead.`;
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
  if (radios.length === 0) throw "Name at least one radio, or pass all.";
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
  return { pack: p.pack, bytes: k.bytes, takes: p.takes ? k.lines : 0, radios: k.radios ?? [] };
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
const SETS: SetInfo[] = [
  { id: "edgetx", title: "Full EdgeTX English", about: "Every prompt the radio plays by itself, the numbers and units, and the general prompts a model can name.", lines: 311 },
  { id: "quad", title: "FPV quad", about: "The radio's own prompts and numbers, and the callouts a quad's radio plays.", lines: 168 },
  { id: "heli", title: "Helicopter", about: "The radio's own prompts and numbers, and the callouts a helicopter's radio plays.", lines: 164 },
  { id: "plane", title: "Plane", about: "The radio's own prompts and numbers, and the callouts a plane's radio plays.", lines: 169 },
  { id: "glider", title: "Glider", about: "The radio's own prompts and numbers, and the callouts a glider's radio plays.", lines: 164 },
  { id: "extras", title: "FPV extras", about: "More FPV callouts: arming states, profiles, VTX, OSD, recording, finder.", lines: 68 },
  { id: "easter", title: "Easter eggs", about: "Short fun lines in original wording. Off unless a model plays them.", lines: 12 },
  { id: "sample", title: "Sample", about: "A dozen hard lines for comparing voices: a bare number, short words, warnings.", lines: 12 },
  { id: "quadcam", title: "QuadCam default", about: "The lines QuadCam's packs hold.", lines: 45 },
  { id: "custom", title: "Your lines", about: "The lines you added.", lines: 0 },
];
const CREDITS = 98_500;
const CARRIER = "The word is .".length;

const keyStatus = (g: MockGear): KeyStatus => (g.voice.keySet ? { set: true, hint: "ends in 3f9a", source: "keychain", problem: null } : { set: false, hint: "", source: "", problem: null });

/** `gear_voice_key`. */
export function key(g: MockGear, p: { action?: string; key?: string | null }): KeyStatus {
  if (p.action === "set") {
    if (!p.key?.trim()) throw "The key is empty.";
    g.voice.keySet = true;
  } else if (p.action === "delete") g.voice.keySet = false;
  return keyStatus(g);
}

/** `gear_voice_sets`. */
export function sets(g: MockGear): StudioView {
  return { key: keyStatus(g), sets: SETS.map((s) => (s.id === "custom" ? { ...s, lines: g.voice.custom.length } : s)), batch: { carrier: "The word is {line}.", tone_carriers: {}, max_lines: 30, snap_ms: 40 } };
}

const needKey = (g: MockGear) => {
  if (!g.voice.keySet) throw "No ElevenLabs key is stored: run `quadcam-cli gear voice key set`, or paste it in the Voice studio.";
};

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

function price(g: MockGear, setIds: string[], model: string): VoiceEstimate {
  const m = MODELS.find((x) => x.id === model);
  if (!m) throw `The account lists no model "${model}".`;
  const lines = SETS.filter((s) => setIds.includes(s.id)).reduce((n, s) => n + s.lines, 0);
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
  const e = price(g, p.sets, p.model);
  return { voice: v.id, voice_name: v.name, model: p.model, lines: e.lines, estimate: e };
}

/** `gear_voice_sample`: three lines per voice and model. */
export function sample(g: MockGear, p: { voices: string[]; models: string[]; dry_run?: boolean; confirm?: boolean }): SampleReport {
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
  const base = { estimate: sum, combos: combos.length, dry_run: !!p.dry_run, warnings: [] as string[] };
  if (p.dry_run) return { ...base, items: [], needs_confirm: false };
  if (!sum.affordable) throw `this render needs ${sum.credits} credits but the account has ${sum.remaining}`;
  if (!p.confirm) return { ...base, items: [], needs_confirm: true };
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
    })),
  );
  return { ...base, items, needs_confirm: false };
}

/** `gear_voice_render` with sets: a batched render into a local pack. */
function studioRender(g: MockGear, p: { voice?: string; sets: string[]; model?: string; dry_run?: boolean; confirm?: boolean }): RenderReport {
  needKey(g);
  const voice = voiceOf(p.voice ?? "");
  const e = price(g, p.sets, p.model ?? "");
  const id = `local-elevenlabs-${voice.name.toLowerCase()}-${(p.model ?? "").replace(/_/g, "-")}`;
  const base = { pack: id, provider: "elevenlabs", voice: voice.name, plan: { lines: e.lines, cached: 0, to_render: e.lines, chars: e.chars }, paid: true, dry_run: !!p.dry_run, notes: [] as string[], estimate: e, warnings: [] as string[] };
  if (p.dry_run) return { ...base, rendered: 0, from_cache: 0, needs_confirm: false };
  if (!e.affordable) throw `this render needs ${e.credits} credits but the account has ${e.remaining}`;
  if (!p.confirm) return { ...base, rendered: 0, from_cache: 0, needs_confirm: true };
  g.voice.spent += e.credits;
  if (!g.voice.installed.includes(id)) g.voice.installed.push(id);
  return { ...base, rendered: e.batches, from_cache: 0, needs_confirm: false };
}
