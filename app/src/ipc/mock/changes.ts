// The mock core's staged changes, FC and card apply, Keep and Revert, and copying settings
// between quads (`core/apply.rs`, `core/apply_card.rs`, `core/bench.rs`), written by hand.
// Settings the mock FC knows are listed here; staging anything else is refused as the core
// does. An apply succeeds unless `failNext` names a line the FC will refuse (or, for a
// card, `failCard` is set: the read-back then fails and the files are put back).
import type { ApplyPlan, ApplyReport, Check, CopyPlan, Edit, StagedChange } from "../types";
import type { DiffItem } from "../types";
import { FC, type MockGear } from "./gear";

/** Settings the mock FC holds, with the value its latest backup shows. */
export const FC_SETTINGS: Record<string, string> = { osd_cap_alarm: "2200", osd_ah_pos: "4281", p_roll: "45", roll_srate: "70" };

/** The settings each part of a copy takes in the mock. */
const PART_SETTINGS: Record<string, string[]> = { osd: ["osd_cap_alarm", "osd_ah_pos"], pid: ["p_roll"], rates: ["roll_srate"] };
const PART_LABEL: Record<string, string> = { rates: "rates", pid: "PID profiles", osd: "OSD", modes: "modes", adjustments: "adjustments", vtx: "VTX", features: "features and beeper" };

/** The radio card's files in the mock: what a card edit changes. */
const RADIO_YML: Record<string, string> = { contrast: "20", vBatWarn: "66", hapticMode: "mode_all" };

const DAY = "2026-10-09";

export interface MockChanges {
  changes: StagedChange[];
  /** A line the FC refuses on the next apply (nothing saved). */
  failNext: string | null;
  /** The next card apply fails its read-back: every file is put back. */
  failCard: boolean;
  /** Settings a quad holds that differ from the default FC's, by device id (for copy). */
  dumps: Record<string, Record<string, string>>;
  /** Each applied change's report (Revert reads the backup and files from it). */
  reports: Record<string, ApplyReport>;
  counter: number;
}

export const freshChanges = (): MockChanges => ({ changes: [], failNext: null, failCard: false, dumps: {}, reports: {}, counter: 0 });

const STAGED = ["draft", "ready", "try", "read_first"];
const staged = (c: StagedChange) => STAGED.includes(c.status);

/** `Pos::encode` with variant 0. */
export const encodePos = (x: number, y: number, profiles: number[]) => (x & 31) | (((x >> 5) & 1) << 10) | ((y & 31) << 5) | (profiles.reduce((m, p) => m | (1 << (p - 1)), 0) << 11);

const lines = (edits: Edit[]): string[] =>
  edits.flatMap((e) => {
    if (e.kind === "fc_set") return [`set ${e.name} = ${e.value}`];
    if (e.kind === "fc_lines") return e.lines.filter((l) => l.trim());
    if (e.kind === "osd_element") return [`set osd_${e.element}_pos = ${encodePos(e.x, e.y, e.profiles)}`];
    return [];
  });

const refused = (code: string, reason: string) => `Refused (${code}): ${reason}`;

const CARD_EDITS = ["model", "radio", "checklist", "model_copy", "model_delete", "restore"];

function validateFc(edits: Edit[]) {
  for (const l of lines(edits.filter((e) => e.kind !== "osd_element"))) {
    if (/^(save|exit|defaults|bl|dfu|batch)\b/.test(l)) throw refused("shape_unknown", `\`${l}\` is not sent; QuadCam saves and exits itself.`);
    const m = /^set\s+(\S+)\s*=\s*(.+)$/.exec(l);
    if (m && !(m[1] in FC_SETTINGS)) throw refused("bad_setting", `\`${m[1]}\` is not a setting on this FC.`);
  }
}

export function stage(g: MockGear, device: string, edits: Edit[], title: string | null, editor: "user" | "agent" = "user", reverts: string | null = null, draft = false): StagedChange {
  const d = g.devices.find((x) => x.id === device);
  if (!d) throw `Unknown device "${device}". See \`gear devices\`.`;
  if (d.kind === "fc") validateFc(edits);
  else if (d.kind === "radio") {
    const bad = edits.find((e) => !CARD_EDITS.includes(e.kind));
    if (bad) throw refused("shape_unknown", `${bad.kind} edits are not a radio card change.`);
  } else throw `Only FC and radio changes can be staged for now; a ${d.kind} change arrives with its package.`;
  const c = g.changeStore;
  c.counter += 1;
  const first = lines(edits)[0] ?? "";
  const only = edits.length === 1 ? edits[0] : null;
  const change: StagedChange = {
    id: `${DAY}-${device}-${c.counter}`,
    device,
    title: title || (only?.kind === "fc_set" ? `Set ${only.name} = ${only.value}` : d.kind === "radio" ? "Radio card change" : `Run \`${first}\``),
    status: draft ? "draft" : "ready",
    edits,
    base_backup: d.last_backup || "",
    editor,
    note: "",
    order: c.changes.filter((x) => x.device === device && staged(x)).length,
    history: [{ at: "2026-10-09T12:00:00Z", status: draft ? "draft" : "ready", note: "Staged" }],
    ...(reverts ? { reverts } : {}),
  };
  c.changes.push(change);
  return change;
}

export function list(g: MockGear, device: string | null, history: boolean): StagedChange[] {
  return structuredClone(g.changeStore.changes.filter((c) => (!device || c.device === device) && (history || staged(c))));
}

export function update(g: MockGear, p: { id: string; title?: string | null; note?: string | null; order?: number | null; status?: string | null; edits?: Edit[] | null }): StagedChange {
  const c = g.changeStore.changes.find((x) => x.id === p.id);
  if (!c) throw `No staged change "${p.id}". See \`gear changes\`.`;
  if (!staged(c)) throw `Change ${p.id} is ${c.status}; only a staged change can be edited.`;
  if (p.title != null) c.title = p.title;
  if (p.note != null) c.note = p.note;
  if (p.order != null) c.order = p.order;
  if (p.edits) c.edits = p.edits;
  if (p.status) {
    if (!["draft", "ready", "try", "read_first"].includes(p.status)) throw `Status ${p.status} is set by the apply engine; set draft, ready, try or read_first, or discard.`;
    if (p.status !== c.status) {
      c.status = p.status as StagedChange["status"];
      (c.history ??= []).push({ at: "2026-10-09T12:05:00Z", status: c.status, note: "" });
    }
  }
  return structuredClone(c);
}

export function discard(g: MockGear, id: string): StagedChange {
  const c = g.changeStore.changes.find((x) => x.id === id);
  if (!c) throw `No staged change "${id}". See \`gear changes\`.`;
  if (!staged(c)) throw `Change ${id} is ${c.status}; only a staged change can be discarded.`;
  c.status = "discarded";
  return structuredClone(c);
}

export function restoreStage(g: MockGear, backup: string, paths: string[] = [], reverts: string | null = null, title: string | null = null): StagedChange {
  const device = backup.split("/")[0];
  const d = g.devices.find((x) => x.id === device);
  if (d?.kind === "radio") {
    if (!paths.length) throw "A card restore names the files to put back (paths): restoring a whole card is not offered.";
    return stage(g, device, [{ kind: "restore", backup, paths }], title || `Restore backup ${backup.split("/")[1]}`, "user", reverts);
  }
  return stage(g, device, [{ kind: "fc_lines", lines: ["set osd_cap_alarm = 2200", "set osd_ah_pos = 4281"] }], title || `Restore backup ${backup.split("/")[1]}`, "user", reverts);
}

const ok = (name: string): Check => ({ name, ok: true });
const fail = (name: string, code: string, reason: string): Check => ({ name, ok: false, refusal: { code: code as never, reason } });

/** The radio card a change would write: plugged in, or unmounted but still in. */
function cardOf(g: MockGear, device: string) {
  return g.connected.find((c) => c.id === device && c.kind === "radio") || g.unmounted.find((c) => c.id === device && c.kind === "radio") || null;
}

/** One model op as a line of the diff. */
const opLine = (op: import("../types").ModelOp): string => {
  switch (op.op) {
    case "set_timer":
      return `timer ${op.index + 1}: ${op.fields.map((f) => `${f.key} ${f.value}`).join(", ")}`;
    case "remove_timer":
      return `timer ${op.index + 1} removed`;
    case "set_screen_values":
      return `screen ${op.index + 1}: ${op.lines.map((l) => l.join(", ")).join(" / ")}`;
    case "set_screen":
      return op.script ? `screen ${op.index + 1}: script ${op.script}` : `screen ${op.index + 1} removed`;
    case "set_logging":
      return op.logging ? `logging: ${op.logging.swtch} every ${op.logging.period_ds / 10} s` : "logging removed";
    case "set_sensor_logs":
      return `log sensors: ${op.sensors.map((s) => `${s.label} ${s.logs ? "on" : "off"}`).join(", ")}`;
    case "set_rf_alarms":
      return `rfAlarms: warning ${op.warning}, critical ${op.critical}`;
    case "set_callout":
      return `callout ${op.callout.track}`;
    case "remove_callout":
      return `callout ${op.track} removed`;
    case "set_checklist":
      return `displayChecklist: ${op.enabled ? 1 : 0}`;
    default:
      return op.op;
  }
};

function cardDiff(c: StagedChange): DiffItem[] {
  const out: DiffItem[] = [];
  const put: string[] = [];
  for (const e of c.edits) {
    if (e.kind === "radio") {
      const l: { op: "add" | "remove" | "same"; text: string }[] = [];
      for (const op of e.ops) {
        if (op.op === "set_scalar") {
          const before = RADIO_YML[op.key];
          if (before != null && before !== op.value) l.push({ op: "remove", text: `${op.key}: ${before}` }, { op: "add", text: `${op.key}: ${op.value}` });
        }
      }
      if (l.length) {
        out.push({ kind: "lines", label: "RADIO/radio.yml", lines: l });
        put.push("RADIO/radio.yml");
      }
    } else if (e.kind === "model") {
      const lines = e.ops.flatMap((op) => (op.op === "rename" ? [{ op: "remove" as const, text: `  name: "${e.name ?? "ALPHA"}"` }, { op: "add" as const, text: `  name: "${op.name}"` }] : [{ op: "add" as const, text: `  ${opLine(op)}` }]));
      out.push({ kind: "lines", label: `MODELS/${e.file}`, lines });
      put.push(`MODELS/${e.file}`);
    } else if (e.kind === "checklist") {
      out.push({ kind: "lines", label: `MODELS/${e.model}.txt`, lines: e.text.split("\n").filter(Boolean).map((text) => ({ op: "add" as const, text })) });
      put.push(`MODELS/${e.model}.txt`);
    } else if (e.kind === "restore") put.push(...e.paths);
  }
  return [{ kind: "files", label: "Card files", put: [...put, ".metadata_never_index"], delete: [] }, ...out];
}

export function plan(g: MockGear, id: string): ApplyPlan {
  const c = g.changeStore.changes.find((x) => x.id === id);
  if (!c) throw `No staged change "${id}". See \`gear changes\`.`;
  if (!staged(c)) throw `Change ${id} is ${c.status}; only a staged change can be applied.`;
  const dev = g.devices.find((d) => d.id === c.device);
  const readFirst: Check = c.status === "read_first" ? fail("Read first", "read_first", "This change is marked Read first. Read the real value on the device, then mark it Ready.") : ok("Read first");
  if (dev?.kind === "radio") {
    const card = cardOf(g, c.device);
    const failedCheck = g.checks.find((k) => k.device === c.device);
    const checks: Check[] = [
      readFirst,
      card ? ok("One card plugged in") : fail("One card plugged in", "no_device", "No radio card is mounted. Plug the radio in (USB Storage) or insert its card."),
      ...(card
        ? [
            ok("Same card as planned"),
            failedCheck?.state === "failed" ? fail("Card check", "card_check", `The card's last check found errors: ${failedCheck.summary}. Repair it first.`) : ok("Card check"),
            ok("Nothing else writing"),
            ok("Known version"),
            ok("Shape understood"),
            ok("Selected model kept"),
            ok("Something to write"),
          ]
        : []),
    ];
    return { change: id, device: { board: "tx16s", firmware: "EdgeTX", version: "2.11.2" }, checks, diff: cardDiff(c), digest: `digest-${id}` };
  }
  const plugged = g.connected.some((x) => x.kind === "fc" && x.link.kind === "serial");
  const checks: Check[] = [
    readFirst,
    plugged ? ok("One FC plugged in") : fail("One FC plugged in", "no_device", "No FC is plugged in. Plug USB in before the battery."),
    ok("Same FC as planned"),
    ok("Known board and version"),
    ok("Backup to compare with"),
    ok("Lines understood"),
    ok("Settings exist"),
    ok("Port free"),
    ok("USB heat"),
  ];
  const diff: DiffItem[] = [
    {
      kind: "lines",
      label: c.title,
      lines: lines(c.edits).flatMap((l) => {
        const m = /^set\s+(\S+)/.exec(l);
        const before = m ? FC_SETTINGS[m[1]] : undefined;
        return before ? [{ op: "remove" as const, text: `set ${m![1]} = ${before}` }, { op: "add" as const, text: l }] : [{ op: "add" as const, text: l }];
      }),
    },
  ];
  return { change: id, device: { board: "BETAFPVG473", firmware: "Betaflight", version: "2025.12.5-alpha" }, checks, diff, digest: `digest-${id}` };
}

/** Records an apply the way `Core::finish_change` does. */
function finish(g: MockGear, c: StagedChange, report: ApplyReport) {
  g.changeStore.reports[c.id] = report;
  const verified = report.status === "verified";
  c.status = verified && c.status === "try" ? "applied" : report.status;
  (c.history ??= []).push({ at: "2026-10-09T12:10:00Z", status: c.status, note: report.message });
  if (verified && c.reverts) {
    const orig = g.changeStore.changes.find((x) => x.id === c.reverts);
    if (orig) orig.status = "reverted";
  }
}

export function apply(g: MockGear, id: string, digest: string): ApplyReport {
  const p = plan(g, id);
  const bad = p.checks.find((c) => !c.ok);
  if (bad?.refusal) throw refused(bad.refusal.code, bad.refusal.reason);
  if (digest !== p.digest) throw refused("before_mismatch", "The plan changed since you read it; plan again.");
  const store = g.changeStore;
  const c = store.changes.find((x) => x.id === id)!;
  const dev = g.devices.find((d) => d.id === c.device);
  const backup = `${c.device}/2026-10-09T120000-before_apply`;
  const base = { change: id, device: c.device, backup, after_backup: null, saved: false, files: [] as string[], notes: [], at: "2026-10-09T12:00:00Z", verify: [] as ApplyReport["verify"] };
  if (dev?.kind === "radio") {
    const files = (p.diff.find((d) => d.kind === "files") as { put: string[] } | undefined)?.put ?? [];
    if (store.failCard) {
      store.failCard = false;
      const r: ApplyReport = {
        ...base,
        status: "failed",
        steps: [
          { name: "Back up", state: "done", detail: backup },
          { name: "Write", state: "failed", detail: "RADIO/radio.yml read back different from what was written; the backed-up bytes are back." },
          { name: "Roll back", state: "done", detail: `${files.length} files put back` },
          { name: "Read back", state: "skipped" },
          { name: "Verify", state: "skipped" },
        ],
        sent: [],
        failed_line: null,
        message: "RADIO/radio.yml read back different from what was written; the backed-up bytes are back. The card is as it was.",
      };
      finish(g, c, r);
      return r;
    }
    for (const e of c.edits) {
      if (e.kind === "radio") for (const op of e.ops) if (op.op === "set_scalar") RADIO_YML[op.key] = op.value;
    }
    const r: ApplyReport = {
      ...base,
      status: "verified",
      steps: [
        { name: "Back up", state: "done", detail: backup },
        { name: "Write", state: "done", detail: `${files.length} written, 0 deleted` },
        { name: "Read back", state: "done" },
        { name: "Verify", state: "done" },
      ],
      sent: [],
      failed_line: null,
      saved: true,
      files,
      after_backup: `${c.device}/2026-10-09T120100-after_apply`,
      message: `Verified: ${files.length} files read back as written.`,
    };
    finish(g, c, r);
    return r;
  }
  const failing = store.failNext && lines(c.edits).find((l) => l === store.failNext);
  if (failing) {
    store.failNext = null;
    const r: ApplyReport = {
      ...base,
      status: "failed",
      steps: [
        { name: "Back up", state: "done", detail: backup },
        { name: "Write", state: "failed", detail: `${failing}: ###ERROR###` },
        { name: "Read back", state: "skipped" },
        { name: "Verify", state: "skipped" },
      ],
      sent: [{ line: failing, text: "###ERROR###", error: true }],
      failed_line: { line: failing, text: "###ERROR###", error: true },
      message: `The FC refused \`${failing}\`. Nothing was saved; the FC is as it was.`,
    };
    finish(g, c, r);
    return r;
  }
  for (const l of lines(c.edits)) {
    const m = /^set\s+(\S+)\s*=\s*(.+)$/.exec(l);
    if (m) FC_SETTINGS[m[1]] = m[2];
  }
  g.devices = g.devices.map((d) => (d.id === c.device ? { ...d, last_backup: `${c.device}/2026-10-09T120100-after_apply` } : d));
  const r: ApplyReport = {
    ...base,
    status: "verified",
    steps: [
      { name: "Back up", state: "done", detail: backup },
      { name: "Write", state: "done" },
      { name: "Read back", state: "done" },
      { name: "Verify", state: "done" },
    ],
    sent: lines(c.edits).map((line) => ({ line, text: "", error: false })),
    failed_line: null,
    saved: true,
    after_backup: `${c.device}/2026-10-09T120100-after_apply`,
    message: "Verified: the FC holds every line as written.",
  };
  finish(g, c, r);
  return r;
}

/** `Core::gear_change_keep`. */
export function keep(g: MockGear, id: string): StagedChange {
  const c = g.changeStore.changes.find((x) => x.id === id);
  if (!c) throw `No staged change "${id}". See \`gear changes\`.`;
  if (c.status !== "applied") throw `Change ${id} is ${c.status}; only an applied Try change waits for Keep or Revert.`;
  c.status = "verified";
  (c.history ??= []).push({ at: "2026-10-09T12:20:00Z", status: "verified", note: "Kept" });
  return structuredClone(c);
}

/** `Core::gear_change_revert`: stages a restore of the backup the apply took. */
export function revert(g: MockGear, id: string, report: ApplyReport | null): StagedChange {
  const c = g.changeStore.changes.find((x) => x.id === id);
  if (!c) throw `No staged change "${id}". See \`gear changes\`.`;
  if (c.status !== "applied" && c.status !== "verified") throw `Change ${id} is ${c.status}; only an applied change can be reverted.`;
  if (g.changeStore.changes.some((x) => x.reverts === id && staged(x))) throw "A revert of this change is already staged.";
  const dev = g.devices.find((d) => d.id === c.device);
  const backup = report?.backup ?? `${c.device}/2026-10-09T120000-before_apply`;
  const paths = dev?.kind === "radio" ? (report?.files?.length ? report.files : ["RADIO/radio.yml"]) : [];
  return restoreStage(g, backup, paths, id, `Revert: ${c.title}`);
}

const hasBackup = (g: MockGear, device: string) => g.devices.some((d) => d.id === device && d.last_backup);

/** The settings a quad holds in the mock: the default FC's, with its own differences. */
export const heldBy = (g: MockGear, device: string): Record<string, string> => ({ ...FC_SETTINGS, ...(g.changeStore.dumps[device] ?? {}) });

/** `Core::gear_copy_plan`. */
export function copyPlan(g: MockGear, p: { from: string; to: string; parts: string[]; settings: string[] }): CopyPlan {
  const to = g.devices.find((d) => d.id === p.to);
  if (!to) throw `Unknown device "${p.to}". See \`gear devices\`.`;
  if (to.kind !== "fc") throw `Settings copy to an FC; "${p.to}" is a ${to.kind}.`;
  const from = p.from.split("/")[0];
  if (from === to.id) throw "That is the same FC; pick another quad to copy from.";
  const src = g.devices.find((d) => d.id === from);
  if (!src || !hasBackup(g, from)) throw `"${p.from}" has no backup to copy from.`;
  if (!hasBackup(g, to.id)) throw `"${p.to}" has no backup yet. Back it up first.`;
  const checks: Check[] = [];
  const [si, ti] = [src.identity ?? {}, to.identity ?? {}];
  const sv = si.version ?? "";
  const tv = ti.version ?? "";
  const rel = (v: string) => v.split(/[.-]/).slice(0, 2).join(".");
  checks.push(si.firmware && si.firmware === ti.firmware ? ok("Same firmware") : fail("Same firmware", "incompatible", `The quads run ${si.firmware ?? "an unknown firmware"} and ${ti.firmware ?? "an unknown firmware"}; QuadCam copies settings between the same firmware only.`));
  checks.push(sv && rel(sv) === rel(tv) ? ok("Same release") : fail("Same release", "incompatible", `The quads run release ${rel(sv) || "unknown"} and ${rel(tv) || "unknown"}; settings change between releases, so QuadCam copies within one release.`));
  checks.push(p.parts.length || p.settings.length ? ok("Something picked") : fail("Something picked", "incompatible", "Pick a part (rates, OSD, modes, ...) or name the settings to copy."));
  const base: CopyPlan = { from_backup: `${from}/2026-10-05T100000-before_apply`, to_backup: to.last_backup || "", checks, edits: [], diff: [], same: 0, skipped: [], notes: [] };
  if (checks.some((c) => !c.ok)) return base;
  const a = heldBy(g, from);
  const b = heldBy(g, to.id);
  const names = [...p.parts.flatMap((x) => PART_SETTINGS[x] ?? []), ...p.settings];
  if (si.board !== ti.board) base.notes.push(`The boards differ (${si.board ?? "unknown"} and ${ti.board ?? "unknown"}): settings tied to the board's chips and buses are left out.`);
  for (const n of [...new Set(names)]) {
    if (!(n in a)) base.skipped.push(`${n}: the source does not hold this setting`);
    else if (!(n in b)) base.skipped.push(`${n}: the target does not hold this setting`);
    else if (a[n] === b[n]) base.same += 1;
    else {
      base.edits.push({ kind: "fc_set", section: { kind: "master" }, name: n, value: a[n] });
      base.diff.push({ op: "remove", text: `set ${n} = ${b[n]}` }, { op: "add", text: `set ${n} = ${a[n]}` });
    }
  }
  checks.push(base.edits.length ? ok("Something differs") : fail("Something differs", "incompatible", "The target already holds everything picked, or none of it applies to it."));
  return base;
}

/** `Core::gear_copy_stage`. */
export function copyStage(g: MockGear, p: { from: string; to: string; parts: string[]; settings: string[] }, editor: "user" | "agent" = "user"): StagedChange {
  const plan = copyPlan(g, p);
  const bad = plan.checks.find((c) => !c.ok);
  if (bad?.refusal) throw refused(bad.refusal.code, bad.refusal.reason);
  const fromName = g.devices.find((d) => d.id === p.from.split("/")[0])?.name || p.from;
  const what = p.parts.length ? p.parts.map((x) => PART_LABEL[x] ?? x).join(", ") : `${p.settings.length} settings`;
  return stage(g, p.to, plan.edits, `Copy ${what} from ${fromName}`, editor);
}

export { FC };
