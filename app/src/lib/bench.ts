// The Bench page's logic (design 7.7): staged changes grouped by device, the first Ready or
// Try item as "Next session", the plug-in prompt, and the queue as Markdown. Pure functions
// over the core's shapes; no store, no calls.
import type { Device, DeviceKind, Edit, StagedChange } from "../ipc/types";
import { deviceName, KIND_LABEL } from "./gear";

/** Statuses a change waits in. */
export const WAITING: StagedChange["status"][] = ["draft", "ready", "try", "read_first"];

export const STATUS_LABEL: Record<StagedChange["status"], string> = {
  draft: "Draft",
  ready: "Ready",
  try: "Try",
  read_first: "Read first",
  applied: "Applied",
  verified: "Verified",
  failed: "Failed",
  reverted: "Reverted",
  discarded: "Discarded",
};

/** What a status means, for the control's hint. */
export const STATUS_HINT: Record<string, string> = {
  draft: "Being edited. The plug-in bar skips it.",
  ready: "Agreed. Apply it when the device is in.",
  try: "Apply, fly, then keep or revert.",
  read_first: "Read the real value on the device before changing anything.",
};

/** What a change does, in one line. */
export function summary(c: StagedChange): string {
  return c.edits
    .map((e: Edit) => {
      if (e.kind === "fc_set") return `set ${e.name} = ${e.value}`;
      if (e.kind === "fc_lines") return e.lines.filter((l) => l.trim()).join("; ");
      if (e.kind === "restore") return e.paths.length ? `restore ${e.paths.join(", ")}` : "restore a backup";
      if (e.kind === "radio") return e.ops.map((o) => (o.op === "set_scalar" ? `${o.key}: ${o.value}` : `select ${o.file}`)).join("; ");
      return e.kind.replace(/_/g, " ");
    })
    .join("; ");
}

/** One device's place on the Bench. */
export interface BenchGroup {
  device: string;
  name: string;
  kind: DeviceKind;
  /** The person named the device. */
  named: boolean;
  /** In the Mac now, mounted or not. */
  present: boolean;
  /** Plugged in, mounted. */
  mounted: boolean;
  /** Unmounted but still plugged in: an apply mounts it. */
  unmounted: boolean;
  /** Staged changes in queue order. */
  staged: StagedChange[];
  /** Applied Try changes waiting for Keep or Revert. */
  awaiting: StagedChange[];
  /** The first Ready or Try change: what the next session applies. */
  next: StagedChange | null;
  /** Changes the plug-in bar would offer (Ready and Try). */
  ready: number;
  /** "Plug in the radio to apply 3 changes.", else null. */
  prompt: string | null;
}

const plural = (n: number, one: string, many: string) => `${n} ${n === 1 ? one : many}`;

/** The device's noun in a prompt: "the radio", "Whoop FC". */
function noun(kind: DeviceKind, name: string, named: boolean): string {
  if (named) return name;
  return kind === "radio" ? "the radio" : `the ${KIND_LABEL[kind]}`;
}

export function plugPrompt(g: Pick<BenchGroup, "ready" | "present" | "kind" | "name" | "named">): string | null {
  if (g.ready === 0 || g.present) return null;
  const how = g.kind === "radio" ? " in USB Storage mode" : "";
  return `Plug in ${noun(g.kind, g.name, g.named)}${how} to apply ${plural(g.ready, "change", "changes")}.`;
}

/** The groups for the Bench: every device with a waiting or an applied-and-undecided
 *  change, in the order of `devices`. `all` holds the staged changes and the history. */
export function benchGroups(all: StagedChange[], devices: Device[], mountedIds: Set<string>, unmountedIds: Set<string>): BenchGroup[] {
  const byDevice = new Map<string, StagedChange[]>();
  for (const c of all) {
    if (!WAITING.includes(c.status) && c.status !== "applied") continue;
    byDevice.set(c.device, [...(byDevice.get(c.device) || []), c]);
  }
  const order = (a: StagedChange, b: StagedChange) => (a.order ?? 0) - (b.order ?? 0) || a.id.localeCompare(b.id);
  const out: BenchGroup[] = [];
  for (const [id, list] of byDevice) {
    const d = devices.find((x) => x.id === id);
    const staged = list.filter((c) => WAITING.includes(c.status)).sort(order);
    const awaiting = list.filter((c) => c.status === "applied").sort(order);
    const kind = d?.kind ?? "fc";
    const g: BenchGroup = {
      device: id,
      name: d ? deviceName(d) : id,
      kind,
      named: !!d?.name?.trim(),
      mounted: mountedIds.has(id),
      unmounted: unmountedIds.has(id),
      present: mountedIds.has(id) || unmountedIds.has(id),
      staged,
      awaiting,
      next: staged.find((c) => c.status === "ready" || c.status === "try") ?? null,
      ready: staged.filter((c) => c.status === "ready" || c.status === "try").length,
      prompt: null,
    };
    g.prompt = plugPrompt(g);
    out.push(g);
  }
  const rank = (g: BenchGroup) => devices.findIndex((d) => d.id === g.device);
  return out.sort((a, b) => rank(a) - rank(b) || a.name.localeCompare(b.name));
}

/** The queue as Markdown, for a note or a chat. */
export function benchMarkdown(groups: BenchGroup[]): string {
  if (!groups.length) return "# Bench\n\nNothing staged.\n";
  const lines = ["# Bench", ""];
  for (const g of groups) {
    lines.push(`## ${g.name} (${KIND_LABEL[g.kind]})`, "");
    if (g.next) lines.push(`Next session: ${g.next.title}`, "");
    for (const c of g.staged) lines.push(`- [ ] **${STATUS_LABEL[c.status]}** ${c.title}${summary(c) ? ` (\`${summary(c)}\`)` : ""}`);
    for (const c of g.awaiting) lines.push(`- [~] **Applied, keep or revert?** ${c.title}`);
    lines.push("");
  }
  return lines.join("\n");
}
