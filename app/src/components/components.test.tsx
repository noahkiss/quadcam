import { act, fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { describe, expect, it, vi } from "vitest";
import { axeViolations } from "../test/axe";
import { limitContextMenu } from "../native";
import { Button } from "./Button";
import { Dialog } from "./Dialog";
import { Checkbox, SearchField, SelectField, TextField } from "./Field";
import { FlagButtons, FlagMark } from "./FlagButton";
import { Menu } from "./Menu";
import { ProgressRing } from "./ProgressRing";
import { SegmentedControl } from "./SegmentedControl";
import { Stars } from "./Stars";
import { Stepper } from "./Stepper";
import { Toast } from "./Toast";
import { toast } from "./toastStore";

describe("Stars", () => {
  it("rates, and the current star again clears", async () => {
    const onRate = vi.fn();
    render(<Stars rating={3} onRate={onRate} />);
    expect(screen.getByRole("img", { name: "3 of 5 stars" })).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "5 stars" }));
    expect(onRate).toHaveBeenLastCalledWith(5);
    await userEvent.click(screen.getByRole("button", { name: "3 stars" }));
    expect(onRate).toHaveBeenLastCalledWith(0);
    expect(screen.getByRole("button", { name: "2 stars" })).toHaveAttribute("aria-pressed", "true");
  });

  it("shows a rating without buttons when read-only", () => {
    render(<Stars rating={2} />);
    expect(screen.getByRole("img", { name: "2 of 5 stars" })).toBeInTheDocument();
    expect(screen.queryByRole("button")).toBeNull();
  });
});

describe("SegmentedControl", () => {
  function Host() {
    const [v, setV] = useState<"grid" | "list">("grid");
    return <SegmentedControl label="View" value={v} onChange={setV} segments={[{ value: "grid", label: "Grid", icon: "grid", iconOnly: true }, { value: "list", label: "List", icon: "list", iconOnly: true }]} />;
  }
  it("presses one segment", async () => {
    render(<Host />);
    expect(screen.getByRole("group", { name: "View" })).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "List" }));
    expect(screen.getByRole("button", { name: "List" })).toHaveAttribute("aria-pressed", "true");
    expect(screen.getByRole("button", { name: "Grid" })).toHaveAttribute("aria-pressed", "false");
  });
});

describe("FlagButtons", () => {
  it("toggles pick and reject", async () => {
    const onFlag = vi.fn();
    const { rerender } = render(<FlagButtons flag="none" onFlag={onFlag} />);
    await userEvent.click(screen.getByRole("button", { name: "Pick" }));
    expect(onFlag).toHaveBeenLastCalledWith("pick");
    rerender(<FlagButtons flag="pick" onFlag={onFlag} />);
    expect(screen.getByRole("button", { name: "Pick", pressed: true })).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Pick" }));
    expect(onFlag).toHaveBeenLastCalledWith("none");
  });
  it("marks a flag", () => {
    render(<><FlagMark flag="pick" /><FlagMark flag="reject" /><FlagMark flag="none" /></>);
    expect(screen.getByTitle("Pick")).toBeInTheDocument();
    expect(screen.getByTitle("Rejected")).toBeInTheDocument();
  });
});

describe("fields", () => {
  it("label every control", async () => {
    const { container } = render(
      <div>
        <TextField label="Name" defaultValue="farm" hint="Short name" />
        <SelectField label="Aircraft" defaultValue="b">
          <option value="a">A</option>
          <option value="b">B</option>
        </SelectField>
        <Checkbox label="Keep originals" detail="In originals/" />
        <SearchField label="Search the library" />
        <Button icon="settings" aria-label="Settings" />
        <ProgressRing value={0.5} label="Copying" />
        <Stepper label="Import steps" current="b" steps={[{ id: "a", label: "Load" }, { id: "b", label: "Review" }]} />
      </div>,
    );
    expect(screen.getByLabelText("Name")).toHaveValue("farm");
    expect(screen.getByLabelText("Name")).toHaveAccessibleDescription("Short name");
    expect(screen.getByLabelText("Aircraft")).toHaveValue("b");
    expect(screen.getByRole("checkbox", { name: /Keep originals/ })).toBeInTheDocument();
    expect(screen.getByRole("searchbox", { name: "Search the library" })).toBeInTheDocument();
    expect(screen.getByRole("progressbar", { name: "Copying" })).toHaveAttribute("aria-valuenow", "50");
    expect(screen.getByText("Review").closest("li")).toHaveAttribute("aria-current", "step");
    expect(await axeViolations(container)).toEqual([]);
  });
});

describe("Dialog", () => {
  it("is named by its title and closes with the button's value", async () => {
    const onClose = vi.fn();
    render(
      <Dialog open title="Erase the card?" onClose={onClose} actions={<><Button type="submit" value="cancel">Cancel</Button><Button type="submit" value="ok">Erase</Button></>}>
        <p>This cannot be undone.</p>
      </Dialog>,
    );
    const dlg = screen.getByRole("dialog", { name: "Erase the card?" });
    expect(dlg).toHaveAttribute("open");
    await userEvent.click(screen.getByRole("button", { name: "Erase" }));
    expect(onClose).toHaveBeenLastCalledWith("ok");
    fireEvent(dlg, new Event("cancel", { cancelable: true }));
    expect(onClose).toHaveBeenLastCalledWith("cancel");
    expect(await axeViolations(document.body)).toEqual([]);
  });

  it("keeps Escape when blocked", () => {
    const onClose = vi.fn();
    render(<Dialog open blockEscape title="Import" onClose={onClose} />);
    fireEvent(screen.getByRole("dialog"), new Event("cancel", { cancelable: true }));
    expect(onClose).not.toHaveBeenCalled();
  });
});

describe("Menu", () => {
  it("moves with arrows and closes with Escape", async () => {
    const onClose = vi.fn();
    const run = vi.fn();
    render(<Menu label="Clip actions" at={{ x: 10, y: 10 }} onClose={onClose} items={[{ label: "Rename", run }, null, { label: "Trash", run, danger: true }]} />);
    const items = screen.getAllByRole("menuitem");
    expect(items[0]).toHaveFocus();
    await userEvent.keyboard("{ArrowDown}");
    expect(items[1]).toHaveFocus();
    await userEvent.keyboard("{ArrowDown}");
    expect(items[0]).toHaveFocus();
    await userEvent.keyboard("{Enter}");
    expect(run).toHaveBeenCalledTimes(1);
    expect(onClose).toHaveBeenCalledWith(true);
    await userEvent.keyboard("{Escape}");
    expect(await axeViolations(document.body)).toEqual([]);
  });
});

describe("toast", () => {
  it("announces in a live region", () => {
    vi.useFakeTimers();
    render(<Toast />);
    act(() => toast("1 cut saved."));
    expect(screen.getByRole("status")).toHaveTextContent("1 cut saved.");
    act(() => vi.advanceTimersByTime(4000));
    expect(screen.getByRole("status")).toHaveTextContent("");
    vi.useRealTimers();
  });
});

describe("limitContextMenu", () => {
  it("allows the web menu only over text fields", () => {
    const off = limitContextMenu();
    document.body.innerHTML = "<h2>Day</h2><input type=text>";
    const fire = (el: Element) => !el.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true }));
    expect(fire(document.querySelector("h2")!)).toBe(true);
    expect(fire(document.querySelector("input")!)).toBe(false);
    off();
  });
});
