// The mock core's sim sync (`Core::gear_sim_sync_plan` and `gear_sim_sync`): plans against the
// recorded sims (a synthetic home read by the real core), refuses while a sim "runs", and
// remembers which profiles a sync wrote so the Rates segment reads them as matching.
import type { ApplyPlan, ApplyReport, BackupSummary, Check, DiffItem, RateAxis, SimRates, SimRestoreParams, SimSyncParams } from "../types";
import * as seed from "./seed";

export interface MockSimSync {
  /** Process names that run; null: the recording's own (Liftoff). */
  running: string[] | null;
  /** `sim:file:profile` of every profile a sync wrote. */
  synced: Set<string>;
  /** The next sync fails its read back: every file is put back. */
  failNext: boolean;
  /** Plans taken and syncs written, for specs. */
  written: string[];
  /** Sims whose file QuadCam backed up (a sync or a restore wrote it). */
  backedUp: Set<string>;
}

export const freshSimSync = (): MockSimSync => ({ running: null, synced: new Set(), failNext: false, written: [], backedUp: new Set() });

const PROCESS: Record<string, string> = { liftoff: "Liftoff", micro: "Liftoff Micro Drones", uncrashed: "Uncrashed", zone: "The Zone" };
const key = (sim: string, file: string, profile: string) => `${sim}:${file}:${profile}`;
const refused = (code: string, reason: string) => `Refused (${code}): ${reason}`;
const ok = (name: string): Check => ({ name, ok: true });
const fail = (name: string, code: string, reason: string): Check => ({ name, ok: false, refusal: { code: code as never, reason } });

/** The sims as the Rates segment reads them now: a synced profile equals the quad. */
export function status(st: MockSimSync, quadKey: string): SimRates[] {
  const list = seed.sims(quadKey);
  const running = st.running;
  for (const s of list) {
    if (running) s.running = running.includes(PROCESS[s.id] ?? s.name);
    for (const f of s.files)
      for (const p of f.profiles) {
        if (p.diff && st.synced.has(key(s.id, f.path, p.name))) {
          p.diff = { ...p.diff, max_diff: p.diff.max_diff.map(() => 0), fit_error: p.diff.fit_error, throttle_differs: p.throttle ? false : null, same: true };
        }
      }
    const reads = s.files.flatMap((f) => f.profiles).filter((p) => p.diff);
    if (quadKey !== "none" && reads.length) s.in_sync = reads.some((p) => p.diff!.same);
  }
  return list;
}

interface Resolved {
  sim: SimRates;
  file: string;
  profile: string;
  supported: boolean;
  axes: RateAxis[];
}

export const plan = (st: MockSimSync, p: SimSyncParams): ApplyPlan => build(st, p).plan;

function build(st: MockSimSync, p: SimSyncParams): { plan: ApplyPlan; picks: Resolved[] } {
  const idx = p.profile ?? 0;
  const view = seed.rates();
  const quad = view.profiles.find((x) => x.index === idx) ?? view.profiles[0];
  const sims = status(st, `p${quad.index}`);
  const checks: Check[] = [];
  const warnings: string[] = [];
  const diff: DiffItem[] = [];
  const picks: Resolved[] = [];
  if (!p.sims.length) checks.push(fail("Sims picked", "incompatible", "Name the sims to sync, or `all`."));
  for (const t of p.sims) {
    const targets = t.sim === "all" ? sims.filter((s) => s.enabled && s.found) : sims.filter((s) => s.id === t.sim);
    if (!targets.length) checks.push(fail(`Sim ${t.sim}`, "shape_unknown", `${JSON.stringify(t.sim)} is not a sim QuadCam knows (liftoff, micro, uncrashed, zone, or all).`));
    for (const s of targets) {
      if (!s.enabled) {
        checks.push(fail(`Adapter on (${s.name})`, "disabled", `The ${s.name} adapter is off.`));
        continue;
      }
      const want = t.profile ?? quad.name ?? "";
      const hit = s.files
        .filter((f) => !t.file || f.path === t.file || f.path.endsWith(`/${t.file}`))
        .flatMap((f) => f.profiles.map((pr) => ({ f, pr })))
        .find(({ pr }) => pr.name.toLowerCase() === want.toLowerCase());
      if (!hit) {
        const names = s.files.flatMap((f) => f.profiles.map((pr) => pr.name)).join(", ");
        checks.push(fail(`File understood (${s.name})`, "shape_unknown", `${s.name} has no profile named ${JSON.stringify(want)}. It has: ${names || "none"}.`));
        continue;
      }
      checks.push(s.running ? fail(`Sim closed (${s.name})`, "sim_running", `Quit ${s.name} first.`) : ok(`Sim closed (${s.name})`));
      if (!hit.pr.supported) {
        checks.push(fail(`File understood (${s.name})`, "shape_unknown", `${hit.pr.name} in ${hit.f.path} holds ${hit.pr.note ?? "rates it cannot read"}; QuadCam writes Betaflight rates only.`));
        continue;
      }
      checks.push(ok(`File understood (${s.name})`), ok(`Rewrites unchanged (${s.name})`), ok(`Writable (${s.name})`));
      const lines: { op: "same" | "add" | "remove"; text: string }[] = [{ op: "same", text: `Profile ${hit.pr.name}` }];
      const same = hit.pr.diff?.same;
      if (same) lines.push({ op: "same", text: "already holds these rates" });
      else
        hit.pr.axes.forEach((a, i) => {
          const q = quad.axes[i];
          for (const [label, a0, q0] of [["RC rate", a.rc_rate, q.rc_rate], ["super rate", a.srate, q.srate], ["expo", a.expo, q.expo]] as const) {
            if (Math.round(a0) !== Math.round(q0)) lines.push({ op: "remove", text: `${["Roll", "Pitch", "Yaw"][i]} ${label} ${Math.round(a0)}` }, { op: "add", text: `${["Roll", "Pitch", "Yaw"][i]} ${label} ${Math.round(q0)}` });
          }
        });
      diff.push({ kind: "lines", label: `${s.name}: ${hit.f.path}`, lines });
      if (!hit.pr.throttle) warnings.push(`${s.name} has no throttle curve: only rates are written.`);
      if (s.id !== "uncrashed") warnings.push(`Unverified: QuadCam read ${s.name}'s file shape from a real install but has not yet seen the game load a file QuadCam wrote. Check the game's rates screen after the sync; the backup holds the old file.`);
      picks.push({ sim: s, file: hit.f.path, profile: hit.pr.name, supported: true, axes: hit.pr.axes });
    }
  }
  if (quad.rates_type !== "betaflight") warnings.push(`${quad.name ?? `rate profile ${quad.index + 1}`} uses the ${quad.rates_type} model; sims take Betaflight rates, so QuadCam fitted them. The largest gap is 40 deg/s.`);
  if (checks.every((c) => c.ok) && picks.length && picks.every((x) => st.synced.has(key(x.sim.id, x.file, x.profile)) || sims.find((s) => s.id === x.sim.id)?.files.flatMap((f) => f.profiles).find((pr) => pr.name === x.profile)?.diff?.same))
    checks.push(fail("Something differs", "incompatible", "The sims already hold these rates; nothing to write."));
  const ready = checks.every((c) => c.ok);
  return {
    picks: ready ? picks : [],
    plan: {
      change: "sim-sync",
      device: {},
      checks,
      diff,
      digest: ready ? `sims-${idx}-${picks.map((x) => key(x.sim.id, x.file, x.profile)).join("|")}` : "",
      warnings: [...new Set(warnings)],
    } as ApplyPlan,
  };
}

export function apply(st: MockSimSync, p: SimSyncParams, digest: string, confirm: boolean): ApplyReport {
  if (!confirm) throw "Refused: sim sync needs the plan's digest and confirm=true.";
  const { plan: pl, picks } = build(st, p);
  const bad = pl.checks.find((c) => !c.ok);
  if (bad?.refusal) throw refused(bad.refusal.code, bad.refusal.reason);
  if (pl.digest !== digest) throw refused("before_mismatch", "The sim files changed since you read the plan; plan again.");
  const keys = picks.map((x) => key(x.sim.id, x.file, x.profile));
  const names = keys.map((k) => k.split(":")[0]);
  const label = (id: string) => seed.sims("none").find((s) => s.id === id)?.name ?? id;
  const base = { change: "sim-sync", device: "sims", backup: `sim-${names[0]}/2026-10-09T120000-before_apply`, after_backup: null, sent: [], failed_line: null, verify: [], notes: pl.warnings ?? [], at: "2026-10-09T12:00:00Z" };
  if (st.failNext) {
    st.failNext = false;
    return {
      ...base,
      status: "failed",
      saved: false,
      files: [],
      steps: [
        ...names.map((n) => ({ name: `Back up ${label(n)}`, state: "done" as const, detail: `sim-${n}/2026-10-09T120000-before_apply` })),
        { name: `Write ${label(names[0])}`, state: "done" as const, detail: keys[0].split(":")[1] },
        { name: `Read back ${label(names[0])}`, state: "failed" as const, detail: "the file does not read as written" },
        { name: "Roll back", state: "done" as const, detail: "1 file(s) put back" },
      ],
      message: "The file does not read back as written. Every file was put back as it was.",
    };
  }
  for (const k of keys) {
    st.synced.add(k);
    st.written.push(k);
    st.backedUp.add(k.split(":")[0]);
  }
  return {
    ...base,
    status: "verified",
    saved: true,
    files: keys.map((k) => k.split(":")[1]),
    steps: keys.flatMap((k) => {
      const n = label(k.split(":")[0]);
      return [
        { name: `Back up ${n}`, state: "done" as const, detail: base.backup },
        { name: `Write ${n}`, state: "done" as const, detail: k.split(":")[1] },
        { name: `Read back ${n}`, state: "done" as const, detail: `${k.split(":")[2]} holds the new rates` },
      ];
    }),
    message: `Verified: ${keys.map((k) => `${label(k.split(":")[0])} (${k.split(":")[2]})`).join(", ")} hold the new rates.`,
  };
}

const BACKUP_AT = "2026-10-09T12:00:00Z";

/** `gear_backups` for a `sim-<id>` device: one backup once QuadCam wrote the sim's file. */
export function backupsOf(st: MockSimSync, device: string): BackupSummary[] {
  const id = device.replace(/^sim-/, "");
  if (!st.backedUp.has(id)) return [];
  return [{ id: `${device}/2026-10-09T120000-before_apply`, device, trigger: "before_apply", taken_at: BACKUP_AT, identity: {}, pinned: false, files: 1, bytes: 2048 } as BackupSummary];
}

const restoreName = (id: string) => seed.sims("none").find((s) => s.id === id)?.name ?? id;

/** `gear_sim_restore_plan`: the backup, the running check, and a digest. */
export function restorePlan(st: MockSimSync, p: SimRestoreParams): ApplyPlan {
  const list = status(st, "none");
  const sim = list.find((s) => s.id === p.sim);
  const checks: Check[] = [];
  const diff: DiffItem[] = [];
  const warnings: string[] = [];
  if (!sim) throw `${JSON.stringify(p.sim)} is not a sim QuadCam knows (liftoff, micro, uncrashed, zone).`;
  if (!st.backedUp.has(sim.id)) {
    checks.push(fail("Backup found", "no_backup", `${sim.name} has no backup: QuadCam backs a sim file up before each sync.`));
  } else {
    checks.push(ok("Backup found"), ok("Backup readable"));
    checks.push(sim.running ? fail(`Sim closed (${sim.name})`, "sim_running", `Quit ${sim.name} first.`) : ok(`Sim closed (${sim.name})`));
    checks.push(ok(`Writable (${sim.name})`));
    const prof = sim.files[0]?.profiles.find((x) => st.synced.has(key(sim.id, sim.files[0].path, x.name)));
    diff.push({ kind: "lines", label: `${sim.name}: ${sim.files[0]?.path ?? ""}`, lines: [{ op: "same", text: `Profile ${prof?.name ?? "?"}` }, { op: "remove", text: "Roll RC rate 127" }, { op: "add", text: "Roll RC rate 100" }] });
    warnings.push("This puts back the file as it was on 2026-10-09 12:00 UTC. Changes made in the game or by a later sync are lost; QuadCam backs the file up first, so this can be undone.");
  }
  const ready = checks.every((c) => c.ok);
  return { change: "sim-restore", device: {}, checks, diff, digest: ready ? `simrestore-${sim.id}` : "", warnings } as ApplyPlan;
}

/** `gear_sim_restore`: puts the file back; the synced profiles read as before. */
export function restoreApply(st: MockSimSync, p: SimRestoreParams, digest: string, confirm: boolean): ApplyReport {
  if (!confirm) throw "Refused: a restore needs the plan's digest and confirm=true.";
  const pl = restorePlan(st, p);
  const bad = pl.checks.find((c) => !c.ok);
  if (bad?.refusal) throw refused(bad.refusal.code, bad.refusal.reason);
  if (pl.digest !== digest) throw refused("before_mismatch", "The sim files or the backup changed since you read the plan; plan again.");
  for (const k of [...st.synced]) if (k.startsWith(`${p.sim}:`)) st.synced.delete(k);
  const n = restoreName(p.sim);
  const backup = `sim-${p.sim}/2026-10-09T120100-before_apply`;
  return {
    change: "sim-restore",
    device: "sims",
    status: "verified",
    saved: true,
    backup,
    after_backup: null,
    sent: [],
    failed_line: null,
    verify: [],
    files: ["~/Library/Application Support/sim.xml"],
    steps: [
      { name: `Back up ${n}`, state: "done", detail: backup },
      { name: `Write ${n}`, state: "done", detail: "~/Library/Application Support/sim.xml" },
      { name: `Read back ${n}`, state: "done", detail: "the file holds the backup's bytes" },
    ],
    message: `Verified: ${n} holds the backed-up file again.`,
    notes: pl.warnings ?? [],
    at: "2026-10-09T12:01:00Z",
  } as ApplyReport;
}
