import { act, fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { describe, expect, it, vi } from "vitest";
import { axeViolations } from "../test/axe";
import { Button } from "./Button";
import { Dialog } from "./Dialog";
import { Checkbox, SearchField, SelectField, TextField } from "./Field";
import { FlagButtons, FlagMark } from "./FlagButton";
import { Menu } from "./Menu";
import { Popover } from "./Popover";
import { PathText } from "./PathText";
import { ProgressRing } from "./ProgressRing";
import { SegmentedControl } from "./SegmentedControl";
import { Stars } from "./Stars";
import { Stepper } from "./Stepper";
import { Toast } from "./Toast";
import { toast } from "./toastStore";
import { useStore } from "../store";

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

  it("Return in a field clicks the primary button, not Cancel", async () => {
    const onClose = vi.fn();
    render(
      <Dialog open title="Rename" onClose={onClose} actions={<><Button type="submit" value="cancel" variant="ghost">Cancel</Button><Button type="submit" value="save" variant="primary">Save</Button></>}>
        <input aria-label="Name" />
      </Dialog>,
    );
    await userEvent.type(screen.getByRole("textbox", { name: "Name" }), "Meadow{Enter}");
    expect(onClose).toHaveBeenCalledTimes(1);
    expect(onClose).toHaveBeenLastCalledWith("save");
  });

  it("Return in a field that handles it itself does not submit", async () => {
    const onClose = vi.fn();
    const search = vi.fn();
    render(
      <Dialog open title="Places" onClose={onClose} actions={<Button type="submit" value="save" variant="primary">Done</Button>}>
        <input aria-label="Search" onKeyDown={(e) => e.key === "Enter" && (e.preventDefault(), search())} />
      </Dialog>,
    );
    await userEvent.type(screen.getByRole("textbox", { name: "Search" }), "pond{Enter}");
    expect(search).toHaveBeenCalledTimes(1);
    expect(onClose).not.toHaveBeenCalled();
  });

  it("Return in a field does nothing while the primary button is disabled or a danger one", async () => {
    const onClose = vi.fn();
    const { rerender } = render(
      <Dialog open title="Rename" onClose={onClose} actions={<><Button type="submit" value="cancel" variant="ghost">Cancel</Button><Button type="submit" value="save" variant="primary" disabled>Save</Button></>}>
        <input aria-label="Name" />
      </Dialog>,
    );
    await userEvent.type(screen.getByRole("textbox", { name: "Name" }), "x{Enter}");
    rerender(
      <Dialog open title="Rename" onClose={onClose} actions={<><Button type="submit" value="cancel" variant="ghost">Cancel</Button><Button type="submit" value="delete" variant="danger">Delete</Button></>}>
        <input aria-label="Name" />
      </Dialog>,
    );
    await userEvent.type(screen.getByRole("textbox", { name: "Name" }), "{Enter}");
    expect(onClose).not.toHaveBeenCalled();
  });

  it("keeps Escape when blocked", () => {
    const onClose = vi.fn();
    render(<Dialog open blockEscape title="Import" onClose={onClose} />);
    fireEvent(screen.getByRole("dialog"), new Event("cancel", { cancelable: true }));
    expect(onClose).not.toHaveBeenCalled();
  });

  it("reports a close the page did not make as Cancel", () => {
    const onClose = vi.fn();
    render(<Dialog open title="Rename" onClose={onClose} />);
    (screen.getByRole("dialog") as HTMLDialogElement).close();
    expect(onClose).toHaveBeenCalledWith("cancel");
  });

  it("opens again after a close the page did not make while blocked", () => {
    const onClose = vi.fn();
    render(<Dialog open blockEscape title="Import" onClose={onClose} />);
    const d = screen.getByRole("dialog") as HTMLDialogElement;
    d.close();
    expect(d.open).toBe(true);
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


describe("PathText", () => {
  it("shows ~, keeps the last segment apart, and the whole path in the title", () => {
    useStore.setState({ home: "/Users/me" });
    const { container } = render(<PathText path="/Users/me/Movies/quadcam" />);
    const el = container.querySelector("[data-component='path']")!;
    expect(el).toHaveTextContent("~/Movies/quadcam");
    expect(el).toHaveAttribute("title", "/Users/me/Movies/quadcam");
    expect(el).toHaveClass("mono", "selectable");
    expect(el.querySelector("[aria-hidden='true']")?.lastElementChild).toHaveTextContent(/^\/quadcam$/);
    expect(el.querySelector("[data-path]")).toHaveAttribute("data-path", "~/Movies/quadcam");
  });
});

describe("Popover", () => {
  it("moves focus to its first control on open", () => {
    const anchor = { current: null };
    render(
      <Popover label="Add place" anchor={anchor} onClose={() => {}}>
        <input aria-label="Place name" />
      </Popover>,
    );
    expect(screen.getByRole("textbox", { name: "Place name" })).toHaveFocus();
  });
});
