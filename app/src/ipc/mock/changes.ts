// The mock core's staged changes and FC apply (`core/apply.rs`), written by hand. Settings
// the mock FC knows are listed here; staging anything else is refused as the core does.
// An apply succeeds unless `failNext` names a line the FC will refuse.
import type { ApplyPlan, ApplyReport, Check, Edit, StagedChange } from "../types";
import type { DiffItem } from "../types";
import { FC, type MockGear } from "./gear";

/** Settings the mock FC holds, with the value its latest backup shows. */
export const FC_SETTINGS: Record<string, string> = { osd_cap_alarm: "2200", osd_ah_pos: "4281", p_roll: "45" };

const DAY = "2026-10-09";

export interface MockChanges {
  changes: StagedChange[];
  /** A line the FC refuses on the next apply (nothing saved). */
  failNext: string | null;
  counter: number;
}

export const freshChanges = (): MockChanges => ({ changes: [], failNext: null, counter: 0 });

const lines = (edits: Edit[]): string[] =>
  edits.flatMap((e) => {
    if (e.kind === "fc_set") return [`set ${e.name} = ${e.value}`];
    if (e.kind === "fc_lines") return e.lines.filter((l) => l.trim());
    return [];
  });

const refused = (code: string, reason: string) => `Refused (${code}): ${reason}`;

function validate(edits: Edit[]) {
  for (const l of lines(edits)) {
    if (/^(save|exit|defaults|bl|dfu|batch)\b/.test(l)) throw refused("shape_unknown", `\`${l}\` is not sent; QuadCam saves and exits itself.`);
    const m = /^set\s+(\S+)\s*=\s*(.+)$/.exec(l);
    if (m && !(m[1] in FC_SETTINGS)) throw refused("bad_setting", `\`${m[1]}\` is not a setting on this FC.`);
  }
}

export function stage(g: MockGear, device: string, edits: Edit[], title: string | null, editor: "user" | "agent" = "user"): StagedChange {
  const d = g.devices.find((x) => x.id === device);
  if (!d) throw `Unknown device "${device}". See \`gear devices\`.`;
  if (d.kind !== "fc") throw "Only FC changes can be staged for now; a radio change arrives with card apply.";
  validate(edits);
  const c = g.changeStore;
  c.counter += 1;
  const first = lines(edits)[0] ?? "";
  const change: StagedChange = {
    id: `${DAY}-${device}-${c.counter}`,
    device,
    title: title || (edits.length === 1 && edits[0].kind === "fc_set" ? `Set ${edits[0].name} = ${edits[0].value}` : `Run \`${first}\``),
    status: "ready",
    edits,
    base_backup: d.last_backup || "",
    editor,
    note: "",
    order: c.changes.filter((x) => x.device === device && x.status === "ready").length,
    history: [{ at: "2026-10-09T12:00:00Z", status: "ready", note: "Staged" }],
  };
  c.changes.push(change);
  return change;
}

export function list(g: MockGear, device: string | null, history: boolean): StagedChange[] {
  return structuredClone(g.changeStore.changes.filter((c) => (!device || c.device === device) && (history || c.status === "ready" || c.status === "draft")));
}

export function discard(g: MockGear, id: string): StagedChange {
  const c = g.changeStore.changes.find((x) => x.id === id);
  if (!c) throw `No staged change "${id}". See \`gear changes\`.`;
  if (c.status !== "ready" && c.status !== "draft") throw `Change ${id} is ${c.status}; only a staged change can be discarded.`;
  c.status = "discarded";
  return structuredClone(c);
}

export function restoreStage(g: MockGear, backup: string): StagedChange {
  const device = backup.split("/")[0];
  return stage(g, device, [{ kind: "fc_lines", lines: ["set osd_cap_alarm = 2200", "set osd_ah_pos = 4281"] }], `Restore backup ${backup.split("/")[1]}`);
}

export function plan(g: MockGear, id: string): ApplyPlan {
  const c = g.changeStore.changes.find((x) => x.id === id);
  if (!c) throw `No staged change "${id}". See \`gear changes\`.`;
  if (c.status !== "ready" && c.status !== "draft") throw `Change ${id} is ${c.status}; only a staged change can be applied.`;
  const plugged = g.connected.some((x) => x.kind === "fc" && x.link.kind === "serial");
  const ok = (name: string): Check => ({ name, ok: true });
  const checks: Check[] = [
    plugged ? ok("One FC plugged in") : { name: "One FC plugged in", ok: false, refusal: { code: "no_device", reason: "No FC is plugged in. Plug USB in before the battery." } },
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

export function apply(g: MockGear, id: string, digest: string): ApplyReport {
  const p = plan(g, id);
  const bad = p.checks.find((c) => !c.ok);
  if (bad?.refusal) throw refused(bad.refusal.code, bad.refusal.reason);
  if (digest !== p.digest) throw refused("before_mismatch", "The plan changed since you read it; plan again.");
  const store = g.changeStore;
  const c = store.changes.find((x) => x.id === id)!;
  const backup = `${c.device}/2026-10-09T120000-before_apply`;
  const base = { change: id, device: c.device, backup, after_backup: null, saved: false, notes: [], at: "2026-10-09T12:00:00Z", verify: [] as ApplyReport["verify"] };
  const failing = store.failNext && lines(c.edits).find((l) => l === store.failNext);
  if (failing) {
    store.failNext = null;
    c.status = "failed";
    return {
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
  }
  for (const l of lines(c.edits)) {
    const m = /^set\s+(\S+)\s*=\s*(.+)$/.exec(l);
    if (m) FC_SETTINGS[m[1]] = m[2];
  }
  c.status = "verified";
  g.devices = g.devices.map((d) => (d.id === c.device ? { ...d, last_backup: `${c.device}/2026-10-09T120100-after_apply` } : d));
  return {
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
}

export { FC };
