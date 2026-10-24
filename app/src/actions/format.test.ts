import { beforeEach, describe, expect, it } from "vitest";
import { api } from "../ipc/api";
import { installMock, type MockHandle } from "../ipc/mock/install";
import { store } from "../store";
import { askFormat, confirmErase, sessionChanged } from "./session";

let mock: MockHandle;
const initial = store.getState();
const S = () => store.getState();
const tick = () => new Promise((r) => setTimeout(r, 0));

async function start(scenario: "finished-card" | "card") {
  store.setState(initial, true);
  mock = installMock({ scenario });
  await S().loadSettings();
  S().setSession(await api.getSession());
}

describe("format after an import", () => {
  beforeEach(() => start("finished-card"));

  it("the mock refuses a format until every clip is in the library", async () => {
    await api.loadSource(mock.core.volumes[0].mount);
    await expect(api.formatPlan(null)).rejects.toMatch(/has not been imported/);
  });

  it("Erase sends the card of the plan the dialog showed", async () => {
    await askFormat();
    const plan = await api.formatPlan(null);
    await confirmErase();
    const sent = mock.calls.filter((c) => c.cmd === "format_card").map((c) => c.args.req);
    expect(sent).toEqual([{ device: plan.device, volume_uuid: plan.volume_uuid, label: plan.label, confirm: true }]);
  });

  it("the mock refuses an erase of another card", async () => {
    await expect(api.formatCard({ device: "/dev/disk9", volume_uuid: "OTHER", label: null, confirm: true })).rejects.toMatch(/does not match/);
  });

  it("an erased session closes the import, whatever its warnings say", () => {
    S().setImportOpen(true);
    const s = S().session!;
    sessionChanged({ ...s, card: null, erased: true, warnings: [] });
    expect(S().session).toBeNull();
    expect(S().importOpen).toBe(false);
  });

  it("a warning that says erased is not an erase", () => {
    const s = S().session!;
    sessionChanged({ ...s, card: null, erased: false, warnings: ["Card was erased"] });
    expect(S().session).not.toBeNull();
  });

  it("the person's own confirm refuses an agent's pending request", async () => {
    store.setState({ agentFormat: 7 });
    await askFormat();
    await tick();
    expect(mock.calls.filter((c) => c.cmd === "answer_format_request").map((c) => c.args)).toEqual([{ id: 7, approve: false }]);
    expect(S().agentFormat).toBeNull();
  });
});
