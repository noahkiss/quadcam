import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { axeViolations } from "../../test/axe";
import { Button } from "../Button";
import { EmptyState } from "./EmptyState";
import { FactList } from "./FactList";
import { MoreMenu } from "./MoreMenu";
import { PageHeader } from "./PageHeader";
import { StatusDot } from "./StatusDot";
import { SummaryLine } from "./SummaryLine";
import { Disclosure } from "./Disclosure";
import { DetailDrawer, WithDrawer } from "./DetailDrawer";

describe("PageHeader", () => {
  it("puts the title, subtitle and actions in one header, the heading under its id", async () => {
    const { container } = render(
      <section aria-labelledby="t">
        <PageHeader id="t" title="Bench" subtitle="3 staged" actions={<Button>Copy</Button>} />
      </section>,
    );
    expect(screen.getByRole("region", { name: "Bench" })).toBeInTheDocument();
    expect(screen.getByRole("heading", { level: 2, name: "Bench" })).toBeInTheDocument();
    expect(screen.getByText("3 staged")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Copy" })).toBeInTheDocument();
    expect(await axeViolations(container)).toEqual([]);
  });
});

describe("EmptyState, FactList and StatusDot", () => {
  it("say what is missing, list facts, and name a state in words", async () => {
    const { container } = render(
      <>
        <EmptyState action={<Button>Add</Button>}>No packs saved.</EmptyState>
        <FactList label="Details" facts={[{ label: "Board", value: "STM32F411", selectable: true }, { label: "Port", value: "/dev/cu.usbmodem0" }]} />
        <StatusDot tone="warn">Update available</StatusDot>
      </>,
    );
    expect(screen.getByText("No packs saved.")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Add" })).toBeInTheDocument();
    expect(screen.getByText("Board").nextElementSibling).toHaveTextContent("STM32F411");
    expect(screen.getByText("Update available")).toBeInTheDocument();
    expect(await axeViolations(container)).toEqual([]);
  });
});

describe("MoreMenu", () => {
  it("opens a menu of the page's rare actions and runs one", async () => {
    const run = vi.fn();
    render(<MoreMenu items={[{ label: "Forget…", danger: true, run }]} />);
    const button = screen.getByRole("button", { name: "More actions" });
    expect(button).toHaveAttribute("aria-expanded", "false");
    await userEvent.click(button);
    await userEvent.click(screen.getByRole("menuitem", { name: "Forget…" }));
    expect(run).toHaveBeenCalledOnce();
    expect(screen.queryByRole("menu")).toBeNull();
  });
});

describe("SummaryLine, Disclosure and DetailDrawer", () => {
  it("joins the counts, drops empty parts, and colours a part that needs action", () => {
    render(<SummaryLine parts={["4 plugged in", null, { text: "1 needs attention", tone: "warn" }, false]} />);
    expect(screen.getByText("4 plugged in")).toBeInTheDocument();
    expect(screen.getByText("1 needs attention").className).toMatch(/warn/);
    expect(screen.getByText("4 plugged in").parentElement!.children).toHaveLength(2);
  });

  it("keeps a disclosure's rows out of the page until it opens", async () => {
    render(
      <Disclosure summary="Passed" count={5}>
        <p>Card space</p>
      </Disclosure>,
    );
    expect(screen.queryByText("Card space")).toBeNull();
    await userEvent.click(screen.getByText("Passed"));
    expect(screen.getByText("Card space")).toBeInTheDocument();
  });

  it("names the drawer by its title and closes it from its button and Escape", async () => {
    const onClose = vi.fn();
    const { container } = render(
      <WithDrawer
        drawer={
          <DetailDrawer title="Pack 3" onClose={onClose}>
            <p>12 flights</p>
          </DetailDrawer>
        }
      >
        <p>Packs</p>
      </WithDrawer>,
    );
    const drawer = screen.getByRole("complementary", { name: "Pack 3" });
    await userEvent.click(screen.getByRole("button", { name: "Close" }));
    expect(onClose).toHaveBeenCalledOnce();
    drawer.querySelector("button")!.focus();
    await userEvent.keyboard("{Escape}");
    expect(onClose).toHaveBeenCalledTimes(2);
    expect(await axeViolations(container)).toEqual([]);
  });
});

describe("PageHeader fold", () => {
  it("shows the fold buttons and puts them, with the rare actions, in the narrow menu", async () => {
    const copy = vi.fn();
    render(<PageHeader title="Bench" fold={[{ label: "Copy settings…", run: copy }]} more={[{ label: "Copy as Markdown", run: () => {} }]} moreLabel="Bench actions" />);
    await userEvent.click(screen.getByRole("button", { name: "Copy settings…" }));
    expect(copy).toHaveBeenCalledOnce();
    // jsdom applies no container query: both menus render; the narrow one holds every action.
    const menus = screen.getAllByRole("button", { name: "Bench actions" });
    expect(menus).toHaveLength(2);
    await userEvent.click(menus[1]);
    expect(screen.getAllByRole("menuitem").map((m) => m.textContent)).toEqual(["Copy settings…", "Copy as Markdown"]);
  });
});
