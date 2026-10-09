// The OSD editor on the mock core (design 7.3, WP7): on a saved FC the OSD segment is the
// editor. Dragging, the arrow keys, the per-profile toggles, the profile copy and the copy
// from another quad each stage through `gear_osd_edit`; the grid shows the layout with the
// staged change on top, with the overlap and off-screen check live; the apply sheet shows
// the CLI line. Axe in both themes.
import AxeBuilder from "@axe-core/playwright";
import type { Locator, Page } from "@playwright/test";
import { expect, test, type AppFixture } from "./fixtures";

const FC_ID = "fc-0a1b2c3d4e5f6071";
const PORT = "/dev/cu.usbmodemFAKE1";

/** The saved FC's OSD segment, with the FC plugged in so the sheet can apply. */
async function openEditor(app: AppFixture, page: Page) {
  await app.open();
  await app.core(`c => c.plug([{ id: "${FC_ID}", kind: "fc", link: { kind: "serial", port: "${PORT}", vid: 1155, pid: 22336, serial_number: null, manufacturer: null, product: null }, identity: { board: "BETAFPVG473" } }])`);
  await page.getByRole("navigation", { name: "Library" }).getByRole("button", { name: /^Devices/ }).click();
  await page.getByRole("list", { name: "Devices" }).getByRole("button", { name: /Whoop FC/ }).click();
  await page.getByRole("group", { name: "Sections" }).getByRole("button", { name: "OSD" }).click();
  await page.getByRole("group", { name: "OSD profile" }).getByRole("button", { name: /^1 RACE/ }).click();
  await expect(page.getByRole("group", { name: /OSD profile 1 on a PAL grid/ })).toBeVisible();
}

const box = (page: Page, element: string) => page.locator(`[role="button"][data-element="${element}"]`);

/** Drags an element by whole cells, measuring a cell from the grid. */
async function dragBy(page: Page, el: Locator, dx: number, dy: number) {
  const grid = (await page.getByRole("group", { name: /^OSD profile/ }).locator("> div").boundingBox())!;
  const cw = grid.width / 30;
  const ch = grid.height / 16;
  const b = (await el.boundingBox())!;
  const x = b.x + b.width / 2;
  const y = b.y + b.height / 2;
  await page.mouse.move(x, y);
  await page.mouse.down();
  await page.mouse.move(x + (dx * cw) / 2, y + (dy * ch) / 2);
  await page.mouse.move(x + dx * cw, y + dy * ch);
  await page.mouse.up();
}

const lastEdit = async (app: AppFixture) => (await app.method("gear_osd_edit")).at(-1);

test("dragging an element stages its move, and the sheet shows the CLI line", async ({ app, page }) => {
  await openEditor(app, page);
  await expect(box(page, "flymode")).toHaveAttribute("aria-label", "Flight mode, column 24, row 14. Arrow keys move it.");
  await dragBy(page, box(page, "flymode"), -3, -2);

  await expect.poll(() => lastEdit(app)).toEqual({ device: FC_ID, moves: [{ element: "flymode", x: 21, y: 12 }], copy: null });
  await expect(box(page, "flymode")).toHaveAttribute("aria-label", "Flight mode, column 21, row 12. Arrow keys move it.");
  await expect(page.getByRole("status").filter({ hasText: "Flight mode is at column 21, row 12" })).toBeVisible();
  const banner = page.getByText("1 OSD edit is staged.");
  await expect(banner).toBeVisible();

  // The view is read again with the staged change on top.
  expect((await app.method("gear_osd")).at(-1)).toMatchObject({ device: FC_ID, staged: true });
  await page.getByRole("button", { name: "Review…" }).click();
  const sheet = page.getByRole("dialog", { name: "Apply to Whoop FC" });
  // x 21 | y 12 << 5 | profiles 1-3 << 11
  await expect(sheet.getByRole("region", { name: "Changes" })).toContainText("set osd_flymode_pos = 14741");
  await expect(sheet.getByRole("list", { name: "Checks" })).toContainText("One FC plugged in");
});

test("the arrow keys move a focused element; Shift moves five cells; edits join one change", async ({ app, page }) => {
  await openEditor(app, page);
  const el = box(page, "flymode");
  await el.focus();
  await page.keyboard.press("ArrowLeft");
  await expect.poll(() => lastEdit(app)).toEqual({ device: FC_ID, moves: [{ element: "flymode", x: 23, y: 14 }], copy: null });
  await expect(el).toHaveAttribute("aria-label", /column 23, row 14/);
  await expect(el).toBeFocused();
  await page.keyboard.press("Shift+ArrowUp");
  await expect.poll(() => lastEdit(app)).toEqual({ device: FC_ID, moves: [{ element: "flymode", x: 23, y: 9 }], copy: null });
  // Two edits of one element are one edit in one change.
  await expect(page.getByText("1 OSD edit is staged.")).toBeVisible();
  expect(await app.method("gear_change_stage")).toHaveLength(0);
  await box(page, "vbat").focus();
  await page.keyboard.press("ArrowUp");
  await expect(page.getByText("2 OSD edits are staged.")).toBeVisible();
  // Moving flymode back to where the FC holds it drops its edit.
  await el.focus();
  await page.keyboard.press("ArrowRight");
  await page.keyboard.press("Shift+ArrowDown");
  await expect(page.getByText("1 OSD edit is staged.")).toBeVisible();
  // Undo discards the change.
  await page.getByRole("button", { name: "Undo OSD edits" }).click();
  await expect(page.getByText(/OSD edits? (is|are) staged/)).toHaveCount(0);
  await expect(el).toHaveAttribute("aria-label", /column 24, row 14/);
});

test("the check follows the edit: overlap and off screen", async ({ app, page }) => {
  await openEditor(app, page);
  // Profile 1 starts with mah_drawn overlapping current.
  await expect(page.getByRole("list", { name: "Problems" })).toContainText("mah_drawn overlaps current");
  // Move current out of the way: the overlap goes.
  await box(page, "current").focus();
  await page.keyboard.press("Shift+ArrowRight");
  await expect(page.getByRole("list", { name: "Problems" })).toHaveCount(0);
  // Push flymode off the right edge: x 29 keeps one of its four columns on screen.
  await box(page, "flymode").focus();
  await page.keyboard.press("Shift+ArrowRight");
  await expect(page.getByRole("list", { name: "Problems" })).toContainText("flymode: 3 cells off screen.");
  await expect(page.getByText("1 problem across the profiles.").or(page.getByText(/problems? across the profiles/))).toBeVisible();
});

test("toggle an element per profile, and set a position in the list", async ({ app, page }) => {
  await openEditor(app, page);
  const list = page.getByRole("table", { name: "Elements in profile 1" });
  const tick = list.getByRole("checkbox", { name: "Show Flight mode in profile 1" });
  await expect(tick).toBeChecked();
  await tick.uncheck();
  await expect.poll(() => lastEdit(app)).toEqual({ device: FC_ID, moves: [{ element: "flymode", profiles: [2, 3] }], copy: null });
  await expect(box(page, "flymode")).toHaveCount(0);
  await expect(page.getByRole("status").filter({ hasText: "Flight mode is at column 24, row 14, shown in profile 2, 3." })).toBeVisible();
  // Another profile still shows it.
  await page.getByRole("group", { name: "OSD profile" }).getByRole("button", { name: /^3/ }).click();
  await expect(box(page, "flymode")).toHaveCount(1);
  await page.getByRole("group", { name: "OSD profile" }).getByRole("button", { name: /^1 RACE/ }).click();
  // Turn an element on that was off in this profile.
  await list.getByRole("checkbox", { name: "Show Altitude in profile 1" }).check();
  await expect.poll(() => lastEdit(app)).toEqual({ device: FC_ID, moves: [{ element: "altitude", profiles: [1] }], copy: null });
  // Type a column.
  const x = list.getByRole("spinbutton", { name: "Altitude X" });
  await x.fill("12");
  await x.press("Enter");
  await expect.poll(() => lastEdit(app)).toEqual({ device: FC_ID, moves: [{ element: "altitude", x: 12 }], copy: null });
  await expect(box(page, "altitude")).toHaveAttribute("aria-label", /column 12, row 1/);
});

test("copy one profile's layout onto another", async ({ app, page }) => {
  await openEditor(app, page);
  const bar = page.getByRole("group", { name: "Copy layout" });
  await expect(bar.getByRole("button", { name: "Copy profile" })).toBeEnabled();
  await bar.getByLabel("Copy from profile").selectOption("1");
  await bar.getByLabel("Copy to profile").selectOption("3");
  await bar.getByRole("button", { name: "Copy profile" }).click();
  await expect.poll(() => lastEdit(app)).toEqual({ device: FC_ID, moves: [], copy: { from: 1, to: 3 } });
  await expect(page.getByRole("status").filter({ hasText: "Profile 3 now shows what profile 1 shows." })).toBeVisible();
  await page.getByRole("group", { name: "OSD profile" }).getByRole("button", { name: /^3/ }).click();
  await expect(box(page, "ah")).toHaveCount(1);
  await expect(box(page, "current")).toHaveCount(1);
  await expect(box(page, "stick_overlay_left")).toHaveCount(0);
  // The same profile cannot be copied onto itself.
  await bar.getByLabel("Copy to profile").selectOption("1");
  await expect(bar.getByRole("button", { name: "Copy profile" })).toBeDisabled();
});

test("copy the layout from another quad opens the copy dialog on OSD", async ({ app, page }) => {
  await openEditor(app, page);
  await page.getByRole("button", { name: "Copy from another quad…" }).click();
  const d = page.getByRole("dialog", { name: "Copy settings" });
  await expect(d.getByRole("checkbox", { name: "OSD" })).toBeChecked();
  await expect(d.getByRole("checkbox", { name: "Rates" })).not.toBeChecked();
  await expect(d.getByLabel("Copy to")).toHaveValue(FC_ID);
});

for (const scheme of ["dark", "light"] as const) {
  test.describe(`${scheme} theme`, () => {
    test.use({ colorScheme: scheme });
    test("the OSD editor passes axe", async ({ app, page }) => {
      await openEditor(app, page);
      await box(page, "flymode").focus();
      await page.keyboard.press("ArrowLeft");
      await expect(page.getByText("1 OSD edit is staged.")).toBeVisible();
      const r = await new AxeBuilder({ page }).withTags(["wcag2a", "wcag2aa", "wcag21aa"]).analyze();
      expect(r.violations.map((v) => `${v.id}: ${v.nodes.map((n) => n.target.join(" ") + " " + n.failureSummary).join(", ")}`)).toEqual([]);
      if (process.env.QC_SHOTS) await page.screenshot({ path: `${process.env.QC_SHOTS}/osd-edit-${scheme}.png`, fullPage: true });
    });
  });
}
