// The mock core's voice (`core/voice.rs`, `gear/voice/`), written by hand: a few of QuadCam's
// lines, one pack in a made-up index, per-line overrides, a local render and Choose voice,
// which stages one `card_files` change as the core does.
import type { Edit, LineOverride, RenderReport, StagedChange, VoiceLine, VoicePack, VoiceView } from "../types";
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
  { path: "SOUNDS/en/0000.wav", text: "zero", group: "numbers", why: "The number 0." },
  { path: "SOUNDS/en/0001.wav", text: "one", group: "numbers", why: "The number 1." },
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
}

export const freshVoice = (): MockVoice => ({ installed: [], index: false, overrides: {}, chosen: {}, custom: [], paid: false });

const AVAILABLE: VoicePack = { id: "en-demo-v1", voice: "Demo", lang: "en", provider: "demo", model: "", lines: LINES.length, license: "CC BY 4.0", attribution: "Made up for the mock core", version: "1", installed: false, local: false, bytes: 3_700_000, stale: false, dir: null };
const LOCAL_ID = "local-say-samantha";

function packs(v: MockVoice): VoicePack[] {
  const out: VoicePack[] = [];
  if (v.installed.includes(LOCAL_ID)) out.push({ ...AVAILABLE, id: LOCAL_ID, voice: "Samantha", provider: "say", license: "Rendered by you; yours to use.", attribution: "", version: "", installed: true, local: true, bytes: 0, dir: `/Users/pilot/gear/voices/${LOCAL_ID}` });
  if (v.installed.includes(AVAILABLE.id)) out.push({ ...AVAILABLE, installed: true, dir: `/Users/pilot/gear/voices/${AVAILABLE.id}` });
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
export function render(g: MockGear, p: { voice?: string; lines?: string[]; dry_run?: boolean; confirm?: boolean }): RenderReport {
  const v = g.voice;
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
