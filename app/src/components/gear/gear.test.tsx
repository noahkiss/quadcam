import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { ChecksList } from "./ChecksList";
import { DeviceHeader } from "./DeviceHeader";
import { DiffView } from "./DiffView";
import { PlugInBar } from "./PlugInBar";

describe("PlugInBar", () => {
  it("shows for a device with staged changes and opens the review", async () => {
    const onReview = vi.fn();
    const { rerender } = render(<PlugInBar count={3} onReview={onReview} />);
    expect(screen.getByRole("status")).toHaveTextContent("3 changes ready");
    await userEvent.click(screen.getByRole("button", { name: "Review…" }));
    expect(onReview).toHaveBeenCalledOnce();
    rerender(<PlugInBar count={1} onReview={onReview} />);
    expect(screen.getByRole("status")).toHaveTextContent("1 change ready");
    rerender(<PlugInBar count={0} onReview={onReview} />);
    expect(screen.queryByRole("status")).toBeNull();
  });
});

describe("DeviceHeader", () => {
  it("names the device, its kind and its state", () => {
    render(<DeviceHeader kind="dvr_card" name="DVR card" state="inserted" />);
    expect(screen.getByRole("heading", { name: "DVR card" })).toBeInTheDocument();
    expect(screen.getByText("Still inserted")).toBeInTheDocument();
  });
  it("says Not connected without a state", () => {
    render(<DeviceHeader kind="fc" name="Whoop FC" />);
    expect(screen.getByText("Not connected")).toBeInTheDocument();
  });
});

describe("ChecksList and DiffView", () => {
  it("shows each check's result and a failure's reason", () => {
    render(<ChecksList checks={[{ name: "Known version", ok: true }, { name: "Same board", ok: false, refusal: { code: "device_changed", reason: "Another board." } }]} />);
    const items = screen.getAllByRole("listitem");
    expect(items[0]).toHaveTextContent("Known versionPassed");
    expect(items[1]).toHaveTextContent("Same boardFailedAnother board.");
  });
  it("shows lines, files and a version pair", () => {
    render(
      <DiffView
        items={[
          { kind: "lines", label: "CLI", lines: [{ op: "remove", text: "set a = 1" }, { op: "add", text: "set a = 2" }] },
          { kind: "files", label: "SD card", put: ["A.wav"], delete: ["B.wav"] },
          { kind: "version", label: "Firmware", before: null, after: "4.5.1" },
        ]}
      />,
    );
    expect(screen.getByRole("region", { name: "CLI" })).toHaveTextContent("− set a = 1 + set a = 2");
    expect(screen.getByRole("region", { name: "SD card" })).toHaveTextContent("Put A.wavDelete B.wav");
    expect(screen.getByRole("region", { name: "Firmware" })).toHaveTextContent("None → 4.5.1");
  });
});
