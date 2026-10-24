import { beforeEach, describe, expect, it } from "vitest";
import { api } from "../ipc/api";
import { installMock, type MockHandle } from "../ipc/mock/install";
import { store } from "../store";
import { importAnyway, runExport } from "./session";

let mock: MockHandle;
const initial = store.getState();
const S = () => store.getState();

async function load() {
  S().setSession(await api.loadSource("/Users/pilot/clips"));
}

describe("a re-import of clips already in the library", () => {
  beforeEach(async () => {
    store.setState(initial, true);
    mock = installMock({ scenario: "empty" });
    await S().loadSettings();
    await load();
    await runExport();
    await load();
  });

  it("skips them, and Import Anyway sends force for those clips only", async () => {
    await runExport();
    const known = S().session!.results.filter((r) => r.in_library).map((r) => r.id);
    expect(known).toEqual([0, 1, 2, 3]);
    expect(S().exportTotal).toBe(4);
    await importAnyway();
    const sent = mock.calls.filter((c) => c.cmd === "import").map((c) => (c.args.params as { force?: boolean }).force);
    expect(sent).toEqual([false, false, true]);
    expect(S().exportTotal).toBe(4);
    expect(S().session!.results.filter((r) => r.in_library)).toEqual([]);
    expect(S().session!.results.filter((r) => r.outcome === "verified").map((r) => r.id)).toEqual([0, 1, 2, 3]);
  });
});
