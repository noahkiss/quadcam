import type { Page } from "@playwright/test";
import { area, dialog, expect, test, type AppFixture } from "../fixtures";

area("import");

const sheet = (page: Page) => dialog(page, "Import");
// A clip row: a list item (legacy) or a grid row (new UI).
const rows = (page: Page) => sheet(page).locator('li, [role="row"]');
const row = (page: Page, name: string) => rows(page).filter({ has: page.getByRole("textbox", { name: `Short name for ${name}` }) });
const suggests = async (app: AppFixture) => (await app.method("suggest")).map((p) => p.patches as { id: number }[]);
const patches = async (app: AppFixture) => (await suggests(app)).flat();
const batches = async (app: AppFixture) => (await suggests(app)).filter((ps) => ps.length > 1);

async function openFolder(app: AppFixture) {
  await app.core("c => c.dialogAnswers.push('/Users/pilot/clips')");
  await app.page.getByRole("button", { name: "Import…" }).first().click();
  await expect(sheet(app.page)).toBeVisible();
  await expect(row(app.page, "PICT0001.AVI")).toBeVisible();
}

test("Import… without a card picks a folder and loads its clips", async ({ app, page }) => {
  await app.open();
  await openFolder(app);
  expect((await app.method("load"))[0]).toEqual({ source: "/Users/pilot/clips" });
  await expect(rows(page).filter({ has: page.getByRole("checkbox") })).toHaveCount(5);
  await expect(sheet(page).getByText("5 clips · 2:10 flying · 0:00 dead air")).toBeVisible();
});

test("a card in the sidebar shows its new clips and loads on click", async ({ app, page }) => {
  await app.open("card");
  const side = page.getByRole("navigation", { name: "Library" });
  await expect(side.getByRole("button", { name: /DVR\s*4 new/ })).toBeVisible();
  await side.getByRole("button", { name: /DVR/ }).click();
  await expect(sheet(page)).toBeVisible();
  expect((await app.method("load"))[0]).toEqual({ source: "/Volumes/DVR" });
});

test("review edits: name, note, date, time and skip go to the core", async ({ app, page }) => {
  await app.open();
  await openFolder(app);
  const name = page.getByRole("textbox", { name: "Short name for PICT0001.AVI" });
  await name.fill("loops");
  await name.press("Tab");
  await expect.poll(() => patches(app)).toContainEqual({ id: 0, name: "loops" });
  await row(page, "PICT0002.AVI").getByPlaceholder("Note").fill("two packs");
  await row(page, "PICT0002.AVI").getByPlaceholder("Note").press("Tab");
  await expect.poll(() => patches(app)).toContainEqual({ id: 1, note: "two packs" });
  await page.getByLabel("Date for PICT0003.AVI").fill("2026-09-26");
  await page.getByLabel("Date for PICT0003.AVI").press("Tab");
  await expect.poll(() => patches(app)).toContainEqual({ id: 2, date: "2026-09-26", time: "17:05" });
  await row(page, "PICT0004.AVI").getByRole("checkbox").check();
  await expect.poll(() => patches(app)).toContainEqual({ id: 3, skip: true });
  await expect(row(page, "PICT0005.AVI").getByRole("checkbox")).toBeDisabled();
});

test("Up and Down move through the clips; S skips the selected one", async ({ app, page }) => {
  await app.open();
  await openFolder(app);
  await expect(row(page, "PICT0001.AVI")).toHaveAttribute("aria-selected", "true");
  await page.keyboard.press("ArrowDown");
  await expect(row(page, "PICT0002.AVI")).toHaveAttribute("aria-selected", "true");
  await page.keyboard.press("s");
  await expect.poll(() => patches(app)).toContainEqual({ id: 1, skip: true });
  await page.keyboard.press("ArrowUp");
  await expect(row(page, "PICT0001.AVI")).toHaveAttribute("aria-selected", "true");
});

test("the session bar sets aircraft, place and date for every clip", async ({ app, page }) => {
  await app.open();
  await openFolder(app);
  await sheet(page).getByLabel("Aircraft").first().selectOption("Five-inch");
  await expect.poll(async () => (await batches(app)).length).toBe(1);
  expect((await batches(app))[0]).toEqual([0, 1, 2, 3].map((id) => ({ id, profile: "Five-inch" })));
  await sheet(page).getByLabel("Place").first().selectOption("Home field");
  await expect.poll(async () => (await batches(app)).length).toBe(2);
});

test("Apply to all clips copies the clip's metadata", async ({ app, page }) => {
  await app.open();
  await openFolder(app);
  await sheet(page).getByRole("button", { name: "Apply to all clips" }).click();
  await expect(page.getByRole("status")).toContainText("Applied to every clip.");
  const all = (await batches(app)).at(-1)!;
  expect(all.map((p) => p.id)).toEqual([1, 2, 3, 4]);
});

test("an agent's suggestion is marked until the person edits it", async ({ app, page }) => {
  await app.open("review");
  await page.getByRole("navigation", { name: "Library" }).getByRole("button", { name: /Unfinished import/ }).click();
  await expect(sheet(page)).toBeVisible();
  await app.core("c => c.agentSuggest(0, { name: 'agent-loops' }, 'Looks like loops in the yard')");
  const name = page.getByRole("textbox", { name: "Short name for PICT0001.AVI" });
  await expect(name).toHaveValue("agent-loops");
  await expect(row(page, "PICT0001.AVI").getByText("agent", { exact: true })).toBeVisible();
  await expect(row(page, "PICT0001.AVI").getByText("Looks like loops in the yard")).toBeVisible();
  await name.fill("my-loops");
  await name.press("Tab");
  await expect(row(page, "PICT0001.AVI").getByText("agent", { exact: true })).toHaveCount(0);
});

test("Command-Return adds to the library, then Finish shows the files", async ({ app, page }) => {
  await app.open();
  await openFolder(app);
  await page.getByRole("textbox", { name: "Short name for PICT0001.AVI" }).fill("loops");
  await page.keyboard.press("Meta+Enter");
  await expect(sheet(page).getByRole("heading", { name: /^Added 4 clips and 1 cut to the library$/ })).toBeVisible();
  expect(await patches(app)).toContainEqual({ id: 0, name: "loops" });
  expect((await app.method("import"))[0]).toMatchObject({ format: "mp4", encoder: "videotoolbox", keep_originals: false });
  await expect(sheet(page).getByText("2026-09-27_loops.mp4")).toBeVisible();
  await sheet(page).getByRole("button", { name: /Add 5 files/ }).click();
  await expect(sheet(page).getByText("5 added to the Drone album.")).toBeVisible();
  await sheet(page).getByRole("button", { name: "Done · show in Library" }).click();
  await expect(sheet(page)).toBeHidden();
  await expect(page.getByRole("article", { name: "loops" })).toBeVisible();
});

test("Escape closes the sheet; the import stays in the sidebar", async ({ app, page }) => {
  await app.open();
  await openFolder(app);
  await page.keyboard.press("Escape");
  await expect(sheet(page)).toBeHidden();
  await page.getByRole("navigation", { name: "Library" }).getByRole("button", { name: /Unfinished import/ }).click();
  await expect(sheet(page)).toBeVisible();
});

test("Start over asks, then clears the session", async ({ app, page }) => {
  await app.open();
  await openFolder(app);
  await sheet(page).getByRole("button", { name: "Start over" }).click();
  const ask = dialog(page, "Start over?");
  await ask.getByRole("button", { name: "Start over" }).click();
  await expect(sheet(page)).toBeHidden();
  expect(await app.method("clear")).toHaveLength(1);
});

test("Format card: unlock, then a dialog names the disk; Return does not erase", async ({ app, page }) => {
  await app.open("finished-card");
  await page.getByRole("button", { name: "Import…" }).first().click();
  const box = sheet(page).getByRole("region", { name: "Format card" });
  await expect(box).toBeVisible();
  await expect(box.getByText("Same card: disk9, volume UUID matches")).toBeVisible();
  const btn = box.getByRole("button", { name: "Format card…" });
  await expect(btn).toBeDisabled();
  await box.getByRole("checkbox", { name: "Unlock" }).check();
  await btn.click();
  const confirm = dialog(page, "Erase the card?");
  await expect(confirm).toContainText("Erase disk9 (DVR,");
  await page.keyboard.press("Enter");
  await expect(confirm).toBeVisible();
  await confirm.getByRole("button", { name: "Erase" }).click();
  await expect.poll(async () => (await app.calls("format_card")).length).toBe(1);
});

test("an agent's format request waits for the person's click", async ({ app, page }) => {
  await app.open("finished-card");
  await page.evaluate(() => window.__qc!.emit("agent-format-request", { id: 7, plan: { disk: "disk9", device: "/dev/disk9", volume_uuid: "u", volume_name: "DVR", size: 31914983424, media_name: "SD Card Reader", clip_count: 5, label: "DVR" } }));
  const confirm = dialog(page, "Erase the card?");
  await expect(confirm.getByText("An agent asked to erase this card.", { exact: false })).toBeVisible();
  await confirm.getByRole("button", { name: "Cancel" }).click();
  await expect.poll(async () => (await app.calls("answer_format_request")).map((c) => c.args)).toEqual([{ id: 7, approve: false }]);
});

test("a folder dropped on the window starts an import", async ({ app, page }) => {
  await app.open();
  await page.evaluate(() => window.__qc!.drop(["/Users/pilot/dropped"]));
  await expect(sheet(page)).toBeVisible();
  expect((await app.calls("load_dropped"))[0].args).toEqual({ paths: ["/Users/pilot/dropped"] });
});

test("the review panel's trim editor sets session cuts; tabs show flight and file", async ({ app, page }) => {
  await app.open();
  await openFolder(app);
  await row(page, "PICT0002.AVI").getByText("2:00").click();
  const inPt = sheet(page).getByRole("textbox", { name: "In point" });
  await inPt.fill("0:50.0");
  await inPt.press("Tab");
  await sheet(page).getByRole("textbox", { name: "Out point" }).fill("1:00.0");
  await sheet(page).getByRole("textbox", { name: "Out point" }).press("Tab");
  await sheet(page).getByRole("button", { name: "Add cut" }).click();
  await expect.poll(async () => (await app.method("session_cuts"))).toContainEqual({
    id: 1, cuts: [{ start: 10, end: 25 }, { start: 50, end: 60 }], removed_cuts: null,
  });
  await sheet(page).getByRole("tab", { name: "File" }).click();
  await expect(sheet(page).getByText("720×480 @ 30.00 fps")).toBeVisible();
  await sheet(page).getByRole("tab", { name: "Flight" }).click();
  await expect(sheet(page).getByText("No radio log for this clip.")).toBeVisible();
});
