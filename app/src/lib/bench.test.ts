import { describe, expect, it } from "vitest";
import type { Device, StagedChange } from "../ipc/types";
import { benchGroups, benchMarkdown, plugPrompt, summary } from "./bench";

const dev = (id: string, kind: Device["kind"], name: string): Device => ({ id, kind, name, aircraft: null, identity: {}, last_seen: null, last_backup: null });
const FC = dev("fc-1", "fc", "Whoop FC");
const RADIO = dev("radio-1", "radio", "");

let n = 0;
const change = (device: string, status: StagedChange["status"], title: string, order = 0, over: Partial<StagedChange> = {}): StagedChange => ({
  id: `c${++n}`,
  device,
  title,
  status,
  edits: [{ kind: "fc_set", section: { kind: "master" }, name: "osd_cap_alarm", value: "1500" }],
  base_backup: "",
  editor: "user",
  note: "",
  order,
  history: [],
  ...over,
});

describe("benchGroups", () => {
  it("groups the waiting and the applied changes by device, in queue order", () => {
    const all = [change("fc-1", "ready", "B", 1), change("fc-1", "try", "A", 0), change("fc-1", "applied", "Flown"), change("fc-1", "verified", "Done"), change("radio-1", "ready", "Card")];
    const g = benchGroups(all, [FC, RADIO], new Set(["fc-1"]), new Set());
    expect(g.map((x) => x.device)).toEqual(["fc-1", "radio-1"]);
    expect(g[0].staged.map((c) => c.title)).toEqual(["A", "B"]);
    expect(g[0].awaiting.map((c) => c.title)).toEqual(["Flown"]);
    expect(g[0].next?.title).toBe("A");
    expect(g[0].ready).toBe(2);
  });

  it("names the next session as the first Ready or Try change, skipping drafts and Read first", () => {
    const all = [change("fc-1", "draft", "D", 0), change("fc-1", "read_first", "R", 1), change("fc-1", "ready", "Go", 2)];
    const [g] = benchGroups(all, [FC], new Set(), new Set());
    expect(g.next?.title).toBe("Go");
    expect(g.ready).toBe(1);
    expect(benchGroups([change("fc-1", "draft", "D")], [FC], new Set(), new Set())[0].next).toBeNull();
  });

  it("prompts to plug in a device that is away, and not one that is in or unmounted", () => {
    const all = [change("radio-1", "ready", "a"), change("radio-1", "ready", "b", 1), change("radio-1", "try", "c", 2)];
    expect(benchGroups(all, [RADIO], new Set(), new Set())[0].prompt).toBe("Plug in the radio in USB Storage mode to apply 3 changes.");
    expect(benchGroups(all, [RADIO], new Set(["radio-1"]), new Set())[0].prompt).toBeNull();
    const unmounted = benchGroups(all, [RADIO], new Set(), new Set(["radio-1"]))[0];
    expect(unmounted.prompt).toBeNull();
    expect(unmounted.unmounted).toBe(true);
    expect(benchGroups([change("fc-1", "ready", "x")], [FC], new Set(), new Set())[0].prompt).toBe("Plug in Whoop FC to apply 1 change.");
  });

  it("leaves out devices with nothing waiting", () => {
    expect(benchGroups([change("fc-1", "verified", "x"), change("fc-1", "discarded", "y")], [FC], new Set(), new Set())).toEqual([]);
  });
});

describe("plugPrompt", () => {
  it("is null with nothing ready", () => {
    expect(plugPrompt({ ready: 0, present: false, kind: "fc", name: "Whoop FC", named: true })).toBeNull();
  });
});

describe("summary and Markdown", () => {
  it("says what a change does in one line", () => {
    expect(summary(change("fc-1", "ready", "t"))).toBe("set osd_cap_alarm = 1500");
    expect(summary(change("radio-1", "ready", "t", 0, { edits: [{ kind: "radio", ops: [{ op: "set_scalar", key: "contrast", value: "25" }] }] }))).toBe("contrast: 25");
    expect(summary(change("radio-1", "ready", "t", 0, { edits: [{ kind: "restore", backup: "b", paths: ["RADIO/radio.yml"] }] }))).toBe("restore RADIO/radio.yml");
  });

  it("exports the queue as a checklist per device", () => {
    const all = [change("fc-1", "ready", "Raise the alarm"), change("fc-1", "applied", "Flown")];
    const md = benchMarkdown(benchGroups(all, [FC], new Set(["fc-1"]), new Set()));
    expect(md).toContain("# Bench");
    expect(md).toContain("## Whoop FC (FC)");
    expect(md).toContain("Next session: Raise the alarm");
    expect(md).toContain("- [ ] **Ready** Raise the alarm (`set osd_cap_alarm = 1500`)");
    expect(md).toContain("- [~] **Applied, keep or revert?** Flown");
    expect(benchMarkdown([])).toBe("# Bench\n\nNothing staged.\n");
  });
});
