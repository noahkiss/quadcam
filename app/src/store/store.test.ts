import { beforeEach, describe, expect, it } from "vitest";
import { installMock, type MockHandle } from "../ipc/mock/install";
import { menuState } from "../menu/useMenu";
import { HISTORY_LIMIT } from "./history";
import { screenOf, selectedIds, visible } from "./library";
import { sameValue } from "./settings";
import { store } from ".";
import * as actions from "../actions/library";

let mock: MockHandle;
const initial = store.getState();
const S = () => store.getState();
const tick = () => new Promise((r) => setTimeout(r, 0));

beforeEach(async () => {
  store.setState(initial, true);
  mock = installMock();
  await S().loadSettings();
  await S().loadLibrary();
});

describe("library slice", () => {
  it("loads the library and picks the screen", async () => {
    expect(S().lib?.clips).toHaveLength(3);
    expect(screenOf(S())).toBe("library");
    S().openDetail("xd0d144c9ce319e86");
    expect(screenOf(S())).toBe("detail");
    S().closeDetail();
    store.setState({ lib: { ...S().lib!, clips: [] } });
    expect(screenOf(S())).toBe("first-run");
  });

  it("selects with Command and Shift like the legacy UI", () => {
    const ids = visible(S()).map((c) => c.id);
    S().select(ids[0]);
    S().select(ids[2], { meta: true });
    expect(selectedIds(S())).toEqual([ids[0], ids[2]]);
    S().select(ids[0]);
    S().select(ids[2], { shift: true });
    expect(selectedIds(S())).toEqual(ids);
    S().select(ids[1], { meta: true });
    expect(selectedIds(S())).toEqual([ids[0], ids[2]]);
  });

  it("drops vanished clips from the selection on reload", async () => {
    S().select("x07fee4b870d01a6f");
    mock.core.trashClips(["x07fee4b870d01a6f"]);
    await S().loadLibrary();
    expect(selectedIds(S())).toEqual([]);
  });
});

describe("history", () => {
  it("undoes and redoes a rating through the core", async () => {
    await actions.rate(["x07fee4b870d01a6f"], 5);
    expect(mock.core.clip("x07fee4b870d01a6f").rating).toBe(5);
    expect(S().done.map((s) => s.label)).toEqual(["Rating"]);
    await actions.undoRedo("undo");
    expect(mock.core.clip("x07fee4b870d01a6f").rating).toBe(2);
    expect(S().undone).toHaveLength(1);
    await actions.undoRedo("redo");
    expect(mock.core.clip("x07fee4b870d01a6f").rating).toBe(5);
  });

  it("clears redo on a new edit and keeps at most 100 steps", async () => {
    await actions.rate(["x07fee4b870d01a6f"], 1);
    await S().undo();
    await actions.rate(["x07fee4b870d01a6f"], 3);
    expect(S().undone).toHaveLength(0);
    for (let i = 0; i < HISTORY_LIMIT + 5; i++) S().pushStep({ label: "x", undo: async () => {}, redo: async () => {} });
    expect(S().done).toHaveLength(HISTORY_LIMIT);
  });

  it("drops a step that fails", async () => {
    S().pushStep({ label: "Bad", undo: () => Promise.reject("gone"), redo: async () => {} });
    await expect(S().undo()).rejects.toBe("gone");
    expect(S().done).toHaveLength(0);
    expect(S().undone).toHaveLength(0);
  });

  it("undoes Move to Trash", async () => {
    const asked = actions.trashClips(["xd0d144c9ce319e86"]);
    await tick();
    S().askReq!.resolve(true);
    await asked;
    expect(mock.core.lib.clips).toHaveLength(2);
    await actions.undoRedo("undo");
    expect(mock.core.lib.clips).toHaveLength(3);
  });
});

describe("settings slice", () => {
  it("compares values without key order", () => {
    expect(sameValue([{ lat: 1, name: "a" }], [{ name: "a", lat: 1 }])).toBe(true);
    expect(sameValue({ a: 1 }, { a: 2 })).toBe(false);
  });

  it("writes only changed keys", async () => {
    const before = structuredClone(S().values);
    await S().saveChanged({ ...before, places: structuredClone(before.places)!.map((p) => ({ name: p.name, lat: p.lat, lon: p.lon })), format: "mov" }, before);
    const writes = mock.calls.filter((c) => c.cmd === "settings_set").map((c) => (c.args.params as { values: object }).values);
    expect(writes).toEqual([{ format: "mov" }]);
  });
});

describe("menu state", () => {
  it("turns clip items on with a selection and checks the view", () => {
    expect(menuState(S(), false).enabled.pick).toBe(false);
    S().select("x07fee4b870d01a6f");
    const m = menuState(S(), false);
    expect(m.enabled.pick).toBe(true);
    expect(m.enabled.rename).toBe(true);
    expect(m.checked["view-grid"]).toBe(true);
    expect(m.checked["sort-date"]).toBe(true);
    expect(m.albumItem).toBe("Add to Drone Album");
    store.setState({ settingsOpen: "library" });
    expect(menuState(S(), false).enabled.pick).toBe(false);
  });
});
