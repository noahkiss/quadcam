import { render, screen, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import type { SimPreset } from "../../../ipc/types";
import { ProfileOptions } from "./ProfileOptions";

const preset = (id: string, fitted: boolean): SimPreset => ({ id, label: id, wheelbase_mm: 75, mass_g: 56, fitted });

describe("ProfileOptions", () => {
  it("lists built-in profiles plainly when none is fitted", () => {
    render(
      <select aria-label="Aircraft">
        <ProfileOptions presets={[preset("meteor75", false), preset("air65ii", false)]} />
      </select>,
    );
    expect(screen.getAllByRole("option").map((o) => o.textContent)).toEqual(["meteor75", "air65ii"]);
    expect(screen.queryByRole("group")).toBeNull();
  });

  it("puts fitted profiles first, under Your quads", () => {
    render(
      <select aria-label="Aircraft">
        <ProfileOptions presets={[preset("meteor75", false), preset("my-whoop", true)]} />
      </select>,
    );
    const [mine, built] = screen.getAllByRole("group");
    expect(mine).toHaveAttribute("label", "Your quads");
    expect(within(mine).getAllByRole("option").map((o) => o.textContent)).toEqual(["my-whoop"]);
    expect(built).toHaveAttribute("label", "Built in");
    expect(within(built).getAllByRole("option").map((o) => o.textContent)).toEqual(["meteor75"]);
  });
});
