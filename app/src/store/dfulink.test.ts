import { beforeEach, describe, expect, it } from "vitest";
import { installMock, type MockHandle } from "../ipc/mock/install";
import { store } from ".";

let mock: MockHandle;
const initial = store.getState();
const S = () => store.getState();
const RADIO = "radio-1f2e3d4c5b6a7980";

describe("the flash sheet's DFU link step", () => {
  beforeEach(async () => {
    store.setState(initial, true);
    mock = installMock({});
    const d = mock.core.gear.devices.find((x) => x.id === RADIO)!;
    d.identity = { board: "pocket", firmware: "EdgeTX", version: "2.12.3" };
    await S().loadGear();
  });

  it("links the radio in DFU mode to the sheet's radio and plans again", async () => {
    await S().openFlash({ device: RADIO, version: "2.12.4", splash: null });
    expect(S().applySheet?.plan?.dfu_link).toBe("3A0F00125431");
    await S().linkDfu();
    expect(mock.calls.filter((c) => c.cmd === "gear_dfu_link").map((c) => c.args.params)).toEqual([{ device: RADIO, serial: "3A0F00125431", unlink: false }]);
    expect(S().applySheet?.plan?.dfu_link).toBeNull();
    expect(S().devices.find((d) => d.id === RADIO)?.dfu_serial).toBe("3A0F00125431");
  });

  it("offers no link on an agent's request", async () => {
    await S().openFlash({ device: RADIO, version: "2.12.4", splash: null });
    store.setState((s) => ({ applySheet: { ...s.applySheet!, agent: 7 } }));
    await S().linkDfu();
    expect(mock.calls.filter((c) => c.cmd === "gear_dfu_link")).toEqual([]);
  });
});
