import { beforeEach, describe, expect, it } from "vitest";
import { installMock, type MockHandle } from "../ipc/mock/install";
import type { ApplyPlan, StagedChange } from "../ipc/types";
import { store } from ".";

let mock: MockHandle;
const initial = store.getState();
const S = () => store.getState();

const change = { id: "volume-label", device: "radio-1f2e3d4c5b6a7980", title: "Rename the radio's volume", status: "ready", edits: [], base_backup: "", editor: "agent", note: "", order: 0, history: [], reverts: null } as unknown as StagedChange;
const plan = { change: "volume-label", device: {}, checks: [{ name: "The radio's card is mounted", ok: true }], diff: [{ kind: "version", label: "Volume name", before: "CARD", after: "POCKET2" }], digest: "d", warnings: [] } as unknown as ApplyPlan;

describe("an agent's device action", () => {
  beforeEach(() => {
    store.setState(initial, true);
    mock = installMock({});
  });

  it("opens the apply sheet, and Apply answers the request without a write of its own", async () => {
    S().agentApply({ id: 4, change, plan });
    expect(S().applySheet?.agent).toBe(4);
    await S().runApply();
    expect(mock.calls.filter((c) => c.cmd === "answer_apply_request").map((c) => c.args)).toEqual([{ id: 4, approve: true }]);
    expect(mock.calls.some((c) => c.cmd.endsWith("_click"))).toBe(false);
    S().agentApplyResult(4, null, "diskutil rename failed: busy");
    expect(S().applySheet?.error).toBe("diskutil rename failed: busy");
  });
});
