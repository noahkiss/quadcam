// The mock core's backups (`core/backup.rs`), written by hand: a few snapshots of the seed
// radio and FC with made-up files, the card check log, the Storage view, prune, export and
// an import of a made-up folder. Behaviour follows the core closely enough for the specs.
import type { BackupContent, BackupResult, BackupSummary, CardCheck, DiffItem, ImportBackupsReport, PruneReport, RepairResult, StorageView } from "../types";
import type { Backup } from "../../bindings";
import { FC, RADIO, type MockGear } from "./gear";

/** A snapshot with its files' text. */
export interface MockBackup {
  backup: Backup;
  text: Record<string, string>;
}

const RADIO_YML = 'semver: 2.11.2\nboard: tx16s\ncurrModel: 1\n';
const MODEL = (n: string, extra = "") => `header:\n  name: "${n}"\ntimers:\n  0:\n    start: 180\n${extra}`;

function snapshot(device: string, at: string, trigger: Backup["trigger"], files: Record<string, string>, pinned = false): MockBackup {
  const stamp = at.replace(/[-:]/g, "").replace(/^(\d{4})(\d{2})(\d{2})T(\d{6}).*$/, "$1-$2-$3T$4");
  return {
    backup: {
      id: `${device}/${stamp}-${trigger}`,
      device,
      trigger,
      taken_at: at,
      identity: (device === RADIO.id ? RADIO.identity : FC.identity) || {},
      files: Object.entries(files).map(([path, t]) => ({ path, size: t.length, xxh64: hash(t), mtime: null })),
      pinned,
    },
    text: files,
  };
}

/** A stand-in content hash (16 hex digits). */
function hash(t: string): string {
  let h = 0x811c9dc5;
  for (let i = 0; i < t.length; i++) h = Math.imul(h ^ t.charCodeAt(i), 0x01000193) >>> 0;
  return h.toString(16).padStart(8, "0").repeat(2);
}

/** Three radio snapshots and two FC snapshots. */
export function seedBackups(): MockBackup[] {
  const radioFiles = { "RADIO/radio.yml": RADIO_YML, "MODELS/model00.yml": MODEL("ALPHA"), "MODELS/model01.yml": MODEL("BRAVO") };
  return [
    snapshot(RADIO.id, "2026-09-20T17:00:00Z", "import", radioFiles),
    snapshot(RADIO.id, "2026-10-01T18:00:00Z", "connect", { ...radioFiles, "MODELS/model01.yml": MODEL("BRAVO", "    countdownBeep: 1\n") }),
    snapshot(RADIO.id, "2026-10-06T18:20:00Z", "manual", { ...radioFiles, "MODELS/model01.yml": MODEL("BRAVO", "    countdownBeep: 2\n"), "SOUNDS/en/hello.wav": "RIFF" }, true),
    snapshot(FC.id, "2026-09-28T09:00:00Z", "connect", { version: "# Betaflight 4.5.1", "diff all": "set osd_ah_pos = 2101\n" }),
    snapshot(FC.id, "2026-10-05T10:00:00Z", "before_apply", { version: "# Betaflight 4.5.1", "diff all": "set osd_ah_pos = 2102\n" }),
  ];
}

const summary = (b: Backup): BackupSummary => ({
  id: b.id,
  device: b.device,
  trigger: b.trigger,
  taken_at: b.taken_at,
  identity: b.identity,
  pinned: !!b.pinned,
  files: b.files.length,
  bytes: b.files.reduce((n, f) => n + f.size, 0),
});

const of = (g: MockGear, device: string) => g.backups.filter((b) => b.backup.device === device).sort((a, b) => a.backup.taken_at.localeCompare(b.backup.taken_at));
const find = (g: MockGear, id: string) => {
  const b = g.backups.find((x) => x.backup.id === id);
  if (!b) throw `No backup "${id}".`;
  return b;
};

export function list(g: MockGear, device: string | null): BackupSummary[] {
  const l = device ? of(g, device) : [...g.backups].sort((a, b) => a.backup.taken_at.localeCompare(b.backup.taken_at));
  return l.map((b) => summary(b.backup)).reverse();
}

export function read(g: MockGear, id: string, path: string | null): BackupContent {
  const b = find(g, id);
  if (!path) return { backup: structuredClone(b.backup), path: null, size: 0, text: null, binary: false };
  const t = b.text[path];
  if (t == null) throw `Backup ${id} has no file "${path}".`;
  const binary = path.endsWith(".wav");
  return { backup: structuredClone(b.backup), path, size: t.length, text: binary ? null : t, binary };
}

function lineDiff(a: string, b: string): DiffItem {
  const x = a.split("\n");
  const y = b.split("\n");
  const lines: { op: "same" | "add" | "remove"; text: string }[] = [];
  x.forEach((l) => lines.push(y.includes(l) ? { op: "same", text: l } : { op: "remove", text: l }));
  y.filter((l) => !x.includes(l)).forEach((l) => lines.push({ op: "add", text: l }));
  return { kind: "lines", label: "", lines: lines.filter((l) => l.text !== "") };
}

export function diff(g: MockGear, a: string, b: string | null): DiffItem[] {
  let older = find(g, a);
  let newer = older;
  if (b) newer = find(g, b);
  else {
    const l = of(g, older.backup.device);
    const i = l.indexOf(older);
    if (i < 1) throw `${a} is the device's first backup; there is nothing before it.`;
    older = l[i - 1];
  }
  const put = Object.keys(newer.text).filter((p) => p !== "status" && older.text[p] !== newer.text[p]);
  const del = Object.keys(older.text).filter((p) => !(p in newer.text));
  const out: DiffItem[] = put.length || del.length ? [{ kind: "files", label: "Files", put, delete: del }] : [];
  for (const p of put) if (p in older.text && !p.endsWith(".wav")) out.push({ ...lineDiff(older.text[p], newer.text[p]), label: p } as DiffItem);
  return out;
}

export function pin(g: MockGear, id: string, pinned: boolean): BackupSummary {
  const b = find(g, id);
  b.backup.pinned = pinned;
  return summary(b.backup);
}

/** `gear_backup`: a new snapshot of the device plugged in, or none when nothing changed. */
export function take(g: MockGear, device: string, now: string): BackupResult {
  const l = of(g, device);
  const last = l[l.length - 1];
  const dev = g.devices.find((d) => d.id === device);
  const report = (b: Backup, isNew: boolean) => ({ backup: structuredClone(b), new: isNew, read: isNew ? b.files.length : 0, skipped: isNew ? 0 : b.files.length, bytes_read: 0, logs: { added: 0, grown: 0, same: 0, kept_both: 0, unchanged: 0 } });
  const kind = device.startsWith("fc-") ? "fc" : "radio";
  const name = dev?.name || `Unnamed ${kind === "fc" ? "FC" : "Radio"}`;
  if (last && g.dirty !== device) return { device, kind, name, report: report(last.backup, false), pruned: null, notes: [] };
  const s = snapshot(device, now, "manual", { ...(last?.text || {}), "MODELS/model01.yml": MODEL("BRAVO", "    countdownBeep: 3\n") });
  g.backups.push(s);
  g.dirty = null;
  if (dev) dev.last_backup = s.backup.id;
  return { device, kind, name, report: report(s.backup, true), pruned: null, notes: [] };
}

export function storage(g: MockGear, gearDir: string): StorageView {
  const devices = [...new Set(g.backups.map((b) => b.backup.device))].sort().map((device) => {
    const l = of(g, device);
    const bytes = l.reduce((n, b) => n + b.backup.files.reduce((m, f) => m + f.size, 0), 0);
    const by: Record<string, number> = {};
    for (const b of l) by[b.backup.trigger] = (by[b.backup.trigger] || 0) + 1;
    const d = g.devices.find((x) => x.id === device);
    return {
      device,
      name: d ? d.name || null : null,
      kind: d?.kind || null,
      snapshots: l.length,
      pinned: l.filter((b) => b.backup.pinned).length,
      by_trigger: by,
      latest: l[l.length - 1]?.backup.taken_at || null,
      manifest_bytes: 2048 * l.length,
      logs: device === RADIO.id ? 14 : 0,
      log_bytes: device === RADIO.id ? 1_800_000 : 0,
      own_blob_bytes: bytes,
      total_bytes: bytes + 2048 * l.length + (device === RADIO.id ? 1_800_000 : 0),
    };
  });
  const blob = devices.reduce((n, d) => n + d.own_blob_bytes, 0) + 41_000_000;
  return {
    gear_dir: gearDir,
    total_bytes: blob + devices.reduce((n, d) => n + d.manifest_bytes + d.log_bytes, 0),
    blobs: 212,
    blob_bytes: blob,
    shared_blob_bytes: 41_000_000,
    manifest_bytes: devices.reduce((n, d) => n + d.manifest_bytes, 0),
    log_bytes: devices.reduce((n, d) => n + d.log_bytes, 0),
    snapshots: g.backups.length,
    devices,
  };
}

/** Keeps the newest of each device's plug-in and manual snapshots, and every pinned
 *  or apply one. */
export function prune(g: MockGear, dryRun: boolean): PruneReport {
  const dropped: string[] = [];
  for (const device of new Set(g.backups.map((b) => b.backup.device))) {
    const thin = of(g, device)
      .filter((b) => !b.backup.pinned && b.backup.trigger !== "before_apply" && b.backup.trigger !== "before_flash")
      .reverse()
      .slice(1);
    dropped.push(...thin.map((b) => b.backup.id));
  }
  if (!dryRun) g.backups = g.backups.filter((b) => !dropped.includes(b.backup.id));
  return { dry_run: dryRun, dropped, kept: g.backups.length - (dryRun ? dropped.length : 0), collected: { blobs: dropped.length, bytes: 12_000 * dropped.length, temp_files: 0 } };
}

/** An import of a made-up folder: two card copies (one the same as the other), an FC pair
 *  and a card no saved radio matches. */
export function importFolder(g: MockGear, folder: string, dryRun: boolean): ImportBackupsReport {
  const logs = { added: 3, grown: 0, same: 0, kept_both: 0, unchanged: 0 };
  const none = { added: 0, grown: 0, same: 0, kept_both: 0, unchanged: 0 };
  const items: ImportBackupsReport["items"] = [
    { path: `${folder}/2026-05-01 radio`, kind: "card", outcome: "imported", device: RADIO.id, identity: RADIO.identity || {}, taken_at: "2026-05-01T12:00:00Z", backup: `${RADIO.id}/2026-05-01T120000-import`, files: 3, logs, reason: null },
    { path: `${folder}/2026-05-08 radio`, kind: "card", outcome: "same", device: RADIO.id, identity: RADIO.identity || {}, taken_at: "2026-05-08T12:00:00Z", backup: `${RADIO.id}/2026-05-01T120000-import`, files: 3, logs: none, reason: "Same as the backup of 2026-05-01." },
    { path: `${folder}/2026-05-01 quad/quad.diff_all.txt`, kind: "fc", outcome: "imported", device: FC.id, identity: FC.identity || {}, taken_at: "2026-05-01T12:00:00Z", backup: `${FC.id}/2026-05-01T120000-import`, files: 2, logs: none, reason: null },
    { path: `${folder}/2026-04-02 other radio`, kind: "card", outcome: "skipped", device: null, identity: { board: "zorro", firmware: "EdgeTX", version: "2.10.5" }, taken_at: "2026-04-02T12:00:00Z", backup: null, files: 0, logs: none, reason: "No saved Radio with board zorro. Plug the device in once so QuadCam knows it, or pass a device." },
  ];
  const known = new Set(g.backups.map((b) => b.backup.id));
  for (const i of items) if (i.outcome === "imported" && known.has(i.backup!)) i.outcome = "same";
  const imported = items.filter((i) => i.outcome === "imported");
  if (!dryRun)
    for (const i of imported) {
      const s = snapshot(i.device!, i.taken_at!, "import", i.kind === "fc" ? { "diff all": "set osd_ah_pos = 2100\n" } : { "RADIO/radio.yml": RADIO_YML, "MODELS/model00.yml": MODEL("ALPHA") });
      g.backups.push(s);
    }
  return {
    folder,
    dry_run: dryRun,
    items,
    imported: imported.length,
    same: items.filter((i) => i.outcome === "same").length,
    skipped: items.filter((i) => i.outcome === "skipped").length,
    logs: imported.length ? logs : none,
    new_devices: [],
  };
}

const check = (device: string, kind: CardCheck["kind"], state: CardCheck["state"], summary: string, at: string, findings: string[] = []): CardCheck => ({
  id: `${device}-${at.replace(/\D/g, "")}`,
  device,
  kind,
  state,
  at,
  seconds: 28,
  fsck_code: state === "ok" ? 0 : 206,
  modified: kind === "repair",
  summary,
  findings,
});

/** `gear_card_check`: the card's scripted next result (`g.cardFails`), logged. */
export function cardCheck(g: MockGear, device: string, now: string): CardCheck {
  const c = g.cardFails.includes(device) ? check(device, "verify", "failed", "The file system has errors. Repair it.", now, ["Warning: Found 2048 orphaned clusters"]) : check(device, "verify", "ok", "The file system is OK.", now);
  g.checks.unshift(c);
  return c;
}

export function repair(g: MockGear, checkId: string, now: string): RepairResult {
  const last = g.checks.find((c) => c.id === checkId);
  if (!last || g.checks.find((c) => c.device === last.device) !== last) throw `Refused: "${checkId}" is not the latest check of a card plugged in. Check the card again.`;
  if (last.state === "ok") throw "Refused: the card's latest check passed; there is nothing to repair.";
  g.cardFails = g.cardFails.filter((d) => d !== last.device);
  const r = check(last.device, "repair", "ok", "Repaired.", now);
  const v = check(last.device, "verify", "ok", "The file system is OK.", now.replace(/Z$/, ".5Z"));
  g.checks.unshift(r);
  g.checks.unshift(v);
  return { backup: `${last.device}/before-repair`, backup_error: null, repair: r, verify: v };
}

/** The latest check of each card plugged in. */
export const latestChecks = (g: MockGear): CardCheck[] => g.connected.flatMap((c) => g.checks.find((x) => x.device === c.id) || []);
