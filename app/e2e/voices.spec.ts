// Gear > Voices on the mock core (design 7.4): the library of packs on this Mac with a sample,
// Delete (asks first, keeps the raw takes unless ticked) and Apply to radios… (one staged
// change per radio, which card is connected now), the index's packs, Render my voice with its
// cost check, an index pack's preview lines, Publish… (the speech check's gate, the review,
// Approve), and axe in both themes.
import AxeBuilder from "@axe-core/playwright";
import type { Page } from "@playwright/test";
import { axeSchemes, dialog, expect, test, type AppFixture } from "./fixtures";

const RADIO_ID = "radio-1f2e3d4c5b6a7980";
const RADIO_LINK = `{ kind: "volume", mount: "/Volumes/RADIO", volume_uuid: null, bus_protocol: "USB", whole_disk: "disk4" }`;
const radio = `{ id: "${RADIO_ID}", kind: "radio", link: ${RADIO_LINK}, identity: { board: "pocket" } }`;

/** Opens Voices; with `more`, two more saved radios (one EdgeTX, one ETHOS) and the first
 *  radio's card plugged in. */
async function openVoices(app: AppFixture, page: Page, more = false) {
  await app.open();
  if (more)
    await app.core(`c => {
      const r = c.gear.devices[0];
      c.gear.devices.push({ ...structuredClone(r), id: "radio-two", name: "Bench radio" });
      c.gear.devices.push({ ...structuredClone(r), id: "radio-ethos", name: "Other radio", identity: { firmware: "ETHOS" } });
      c.plug([${radio}]);
    }`);
  await page.getByRole("navigation", { name: "Library" }).getByRole("button", { name: /^Voices/ }).click();
}

async function installDemo(page: Page) {
  await page.getByRole("button", { name: "Refresh packs" }).click();
  await page.getByRole("button", { name: "Install Demo" }).click();
  await expect(library(page)).toContainText("Demo");
}

const library = (page: Page) => page.getByRole("table", { name: "Voice packs on this Mac" });

test("the library lists each pack with its model, line sets, lines, size and date, and plays a sample", async ({ app, page }) => {
  await openVoices(app, page);
  await expect(page.getByRole("region", { name: "Library" })).toContainText("No voice pack on this Mac.");
  await expect(page.getByRole("region", { name: "Get voice packs" })).not.toContainText("Demo");
  await page.getByRole("button", { name: "Refresh packs" }).click();
  expect((await app.method("gear_voice")).some((p) => p.refresh_index === true && p.radio === null)).toBe(true);
  await expect(page.getByRole("list", { name: "Packs in the index" })).toContainText("Demo (en-demo-v1) · demo · 7 lines · 2 MB · CC BY 4.0");
  await page.getByRole("button", { name: "Install Demo" }).click();
  expect((await app.method("gear_voice_pack_install")).at(-1)).toEqual({ pack: "en-demo-v1", source: null });
  const row = library(page).getByRole("row", { name: /^Demo en-demo-v1/ });
  await expect(row).toContainText("QuadCam default");
  await expect(row).toContainText("7");
  await expect(row).toContainText("4 MB");
  await expect(row).toContainText("2026");
  await row.getByRole("button", { name: "Play a sample of Demo" }).click();
  expect((await app.method("gear_voice_preview")).at(-1)).toEqual({ line: "SOUNDS/en/armed.wav", pack: "en-demo-v1", radio: null });
});

test("Delete asks first, and keeps the raw takes unless the box is ticked", async ({ app, page }) => {
  await openVoices(app, page);
  await page.getByRole("button", { name: "Render my voice" }).click();
  await installDemo(page);
  await library(page).getByRole("button", { name: "Delete Samantha" }).click();
  const confirm = dialog(page, "Delete Samantha?");
  await expect(confirm).toContainText("Removes local-say-samantha");
  await confirm.getByRole("button", { name: "Cancel" }).click();
  expect(await app.method("gear_voice_pack_delete")).toEqual([]);
  await expect(library(page)).toContainText("Samantha");
  await library(page).getByRole("button", { name: "Delete Samantha" }).click();
  await confirm.getByRole("button", { name: "Delete" }).click();
  expect((await app.method("gear_voice_pack_delete")).at(-1)).toEqual({ pack: "local-say-samantha", takes: false });
  await expect(library(page)).not.toContainText("Samantha");
  await library(page).getByRole("button", { name: "Delete Demo" }).click();
  await dialog(page, "Delete Demo?").getByRole("checkbox", { name: /Also delete the raw takes/ }).check();
  await dialog(page, "Delete Demo?").getByRole("button", { name: "Delete" }).click();
  expect((await app.method("gear_voice_pack_delete")).at(-1)).toEqual({ pack: "en-demo-v1", takes: true });
  await expect(page.getByRole("region", { name: "Library" })).toContainText("No voice pack on this Mac.");
});

test("Delete names the radios that chose the pack and clears their choice", async ({ app, page }) => {
  await openVoices(app, page, true);
  await page.getByRole("button", { name: "Render my voice" }).click();
  await expect(library(page)).toContainText("Samantha");
  await app.core(`c => { c.gear.voice.chosen = { "${RADIO_ID}": "local-say-samantha", "radio-two": "local-say-samantha" }; }`);
  await page.getByRole("button", { name: "Refresh packs" }).click();
  await library(page).getByRole("button", { name: "Delete Samantha" }).click();
  const confirm = dialog(page, "Delete Samantha?");
  await expect(confirm).toContainText("Chosen for Field radio, Bench radio: the delete clears that choice.");
  await confirm.getByRole("button", { name: "Delete" }).click();
  await expect(library(page)).toHaveCount(0);
  expect(await app.core(`c => c.gear.voice.chosen`)).toEqual({});
});

test("Apply to radios stages one change per radio, says which cards are connected, and opens the apply sheet", async ({ app, page }) => {
  await openVoices(app, page, true);
  await installDemo(page);
  await library(page).getByRole("button", { name: "Apply to radios…" }).click();
  const sheet = dialog(page, "Apply Demo to radios");
  const radios = sheet.getByRole("group", { name: "Radios" });
  // EdgeTX radios only.
  await expect(radios.getByRole("checkbox")).toHaveCount(3);
  await expect(radios).not.toContainText("Other radio");
  await expect(radios.getByRole("checkbox", { name: /Field radio Card connected/ })).toBeVisible();
  await expect(radios.getByRole("checkbox", { name: /Bench radio Not connected/ })).toBeVisible();
  await expect(sheet.getByRole("button", { name: "Stage" })).toBeDisabled();
  await radios.getByRole("checkbox", { name: "All radios" }).check();
  await expect(radios.getByRole("checkbox", { name: /Bench radio/ })).toBeChecked();
  await radios.getByRole("checkbox", { name: /Bench radio/ }).uncheck();
  await expect(radios.getByRole("checkbox", { name: "All radios" })).not.toBeChecked();
  await radios.getByRole("checkbox", { name: "All radios" }).check();
  await sheet.getByRole("button", { name: "Stage" }).click();
  expect((await app.method("gear_voice_choose_radios")).at(-1)).toEqual({ pack: "en-demo-v1", radios: [], all: true, keep_overrides: true, editor: null });
  const staged = sheet.getByRole("list", { name: "Staged" });
  await expect(staged).toContainText("Field radio: staged.");
  await expect(staged).toContainText("Bench radio: staged. It applies when you plug the radio in.");
  await expect(staged.getByRole("button", { name: "Review Bench radio" })).toHaveCount(0);
  const changes = await app.core<{ title: string; device: string }[]>(`c => c.dispatch("gear_changes", { device: null, history: false })`);
  expect(changes.map((c) => [c.device, c.title]).sort()).toEqual([
    ["radio-1f2e3d4c5b6a7980", "Voice: Demo"],
    ["radio-two", "Voice: Demo"],
  ]);
  await expect(library(page).getByRole("row", { name: /^Demo/ })).toContainText("Field radio, Bench radio");
  await staged.getByRole("button", { name: "Review Field radio" }).click();
  const apply = page.getByRole("dialog", { name: "Apply to Field radio" });
  await expect(apply.getByRole("region", { name: "Changes" })).toContainText("SOUNDS/en/armed.wav");
  await apply.getByRole("button", { name: "Apply" }).click();
  await expect(apply.getByRole("region", { name: "Result" })).toContainText("Verified");
});

test("Render my voice checks the cost first, and a provider that may charge waits for the click", async ({ app, page }) => {
  await openVoices(app, page);
  const get = page.getByRole("region", { name: "Get voice packs" });
  await get.getByRole("button", { name: "Check cost" }).click();
  await expect(get.getByRole("status")).toContainText("Would render 7 of 7 lines with say; 0 come from the cache; 46 characters.");
  await app.core(`c => { c.gear.voice.paid = true; }`);
  await get.getByRole("button", { name: "Render my voice" }).click();
  await expect(get.getByRole("status")).toContainText("Not rendered: 46 characters would go to say, which may charge for them.");
  expect((await app.method("gear_voice_render")).at(-1)).toMatchObject({ confirm: false, dry_run: false });
  await get.getByRole("button", { name: "Render and pay" }).click();
  expect((await app.method("gear_voice_render")).at(-1)).toMatchObject({ confirm: true });
  await expect(get.getByRole("status")).toContainText("Rendered 7 of 7 lines into local-say-samantha");
  await expect(library(page).getByRole("row", { name: /^Samantha local-say-samantha/ })).toContainText("Rendered here");
});

test("the speech check says why it is unavailable and installs the speech assets on request", async ({ app, page }) => {
  await openVoices(app, page);
  await page.getByRole("button", { name: "Render my voice" }).click();
  await app.core(`c => { c.gear.voice.speech = false; }`);
  await page.getByRole("button", { name: "Refresh packs" }).click();
  const row = library(page).getByRole("row", { name: /^Samantha local-say-samantha/ });
  await expect(row).toContainText("Not checked");
  await row.getByRole("button", { name: "Check speech of Samantha" }).click();
  const check = page.getByRole("region", { name: "Speech check" });
  await expect(check).toContainText("The speech check is unavailable: The speech assets for en-US are not installed");
  await expect(check.getByRole("button", { name: "Check speech" })).toBeDisabled();
  await check.getByRole("button", { name: "Install speech assets" }).click();
  expect((await app.method("gear_voice_check")).at(-1)).toEqual({ pack: "local-say-samantha", lines: [], accept: [], install: true });
  await expect(check.getByRole("status")).toContainText("The speech assets for en-US are installed.");
  await expect(row).toContainText("Heard right");
  await expect(check.getByRole("table", { name: "Speech check of Samantha" }).getByRole("row", { name: /^Armed/ })).toContainText("Heard right");
  await check.getByRole("button", { name: "Close" }).click();
  await expect(check).toHaveCount(0);
});

test("an index pack plays its preview lines before an install", async ({ app, page }) => {
  await openVoices(app, page);
  await page.getByRole("button", { name: "Refresh packs" }).click();
  await page.getByRole("button", { name: "Preview Demo" }).click();
  expect((await app.method("gear_voice_pack_preview")).at(-1)).toEqual({ pack: "en-demo-v1", source: null });
  const lines = page.getByRole("list", { name: "Preview lines of Demo" });
  await expect(lines.getByRole("button")).toHaveText(["Armed", "Disarmed", "Battery low"]);
  await lines.getByRole("button", { name: "Armed", exact: true }).click();
  await page.getByRole("button", { name: "Preview Demo" }).click();
  await expect(lines).toHaveCount(0);
  expect(await app.method("gear_voice_pack_preview")).toHaveLength(1);
});

const STUDIO = "local-elevenlabs-voice-callum-eleven_v3";

/** A studio pack whose Battery low the speech check hears wrong, accepted on the Speech check. */
async function checkedPack(app: AppFixture, page: Page) {
  await openVoices(app, page);
  await app.core(`c => { c.gear.voice.installed.push("${STUDIO}"); c.gear.voice.misheard = "SOUNDS/en/lowbat.wav"; }`);
  await page.getByRole("button", { name: "Refresh packs" }).click();
  await library(page).getByRole("button", { name: "Check speech of Callum" }).click();
  const check = page.getByRole("region", { name: "Speech check" });
  await check.getByRole("button", { name: "Check speech" }).click();
  await check.getByRole("button", { name: "Accept what was heard for Battery low" }).click();
  await check.getByRole("button", { name: "Close" }).click();
}

test("Publish needs the speech check, reviews every line, and publishes only on Approve", async ({ app, page }) => {
  await openVoices(app, page);
  await app.core(`c => { c.gear.voice.installed.push("${STUDIO}"); }`);
  await page.getByRole("button", { name: "Refresh packs" }).click();
  await library(page).getByRole("button", { name: "Publish Callum" }).click();
  const pub = page.getByRole("region", { name: "Publish" });
  await pub.getByRole("button", { name: "Prepare" }).click();
  await expect(pub.getByRole("alert")).toContainText("The speech check has not heard 7 of the pack's 7 lines");
  await expect(pub.getByRole("table")).toHaveCount(0);
  await pub.getByRole("button", { name: "Close" }).click();

  await checkedPack(app, page);
  await library(page).getByRole("button", { name: "Publish Callum" }).click();
  await pub.getByRole("textbox", { name: "Version" }).fill("2");
  await pub.getByRole("button", { name: "Prepare" }).click();
  expect((await app.method("gear_voice_publish")).at(-1)).toEqual({ pack: STUDIO, id: null, version: "2", voice_name: null, license: null, attribution: null, repo: null });
  await expect(pub).toContainText("Iden-callum v2");
  const every = pub.getByRole("table", { name: "Every line" });
  await expect(every.getByRole("row").nth(1)).toContainText("Battery low");
  await expect(every.getByRole("row").nth(1)).toContainText("Accepted");
  await expect(pub.getByRole("table", { name: "Spot check" }).getByRole("row")).toHaveCount(8);
  await every.getByRole("button", { name: "Play Armed" }).click();
  expect(await app.method("gear_voice_publish_approve")).toEqual([]);
  await pub.getByRole("button", { name: "Approve and publish" }).click();
  expect((await app.method("gear_voice_publish_approve")).at(-1)).toEqual({ digest: "mock-en-callum-2", id: "en-callum" });
  await expect(pub).toContainText("Released en-callum v2.");
  await expect(pub.getByRole("button", { name: "Open the pull request" })).toBeVisible();
  await expect(pub.getByRole("button", { name: "Approve and publish" })).toHaveCount(0);
});

test("a pack made with say cannot be published", async ({ app, page }) => {
  await openVoices(app, page);
  await page.getByRole("button", { name: "Render my voice" }).click();
  await library(page).getByRole("button", { name: "Publish Samantha" }).click();
  const pub = page.getByRole("region", { name: "Publish" });
  await pub.getByRole("button", { name: "Prepare" }).click();
  await expect(pub.getByRole("alert")).toContainText("made with macOS say");
});

for (const scheme of axeSchemes) {
  test.describe(`${scheme} theme`, () => {
    test.use({ colorScheme: scheme });
    test("the Voices page passes axe", async ({ app, page }) => {
      await openVoices(app, page, true);
      await installDemo(page);
      let r = await new AxeBuilder({ page }).withTags(["wcag2a", "wcag2aa", "wcag21aa"]).analyze();
      expect(r.violations.map((v) => `${v.id}: ${v.nodes.map((n) => n.target.join(" ") + " " + n.failureSummary).join(", ")}`)).toEqual([]);
      await library(page).getByRole("button", { name: "Apply to radios…" }).click();
      await expect(dialog(page, "Apply Demo to radios")).toBeVisible();
      r = await new AxeBuilder({ page }).withTags(["wcag2a", "wcag2aa", "wcag21aa"]).analyze();
      expect(r.violations.map((v) => `${v.id}: ${v.nodes.map((n) => n.target.join(" ") + " " + n.failureSummary).join(", ")}`)).toEqual([]);
      await dialog(page, "Apply Demo to radios").getByRole("button", { name: "Cancel" }).click();
      await library(page).getByRole("button", { name: "Check speech of Demo" }).click();
      await page.getByRole("region", { name: "Speech check" }).getByRole("button", { name: "Check speech" }).click();
      await expect(page.getByRole("table", { name: "Speech check of Demo" })).toBeVisible();
      r = await new AxeBuilder({ page }).withTags(["wcag2a", "wcag2aa", "wcag21aa"]).analyze();
      expect(r.violations.map((v) => `${v.id}: ${v.nodes.map((n) => n.target.join(" ") + " " + n.failureSummary).join(", ")}`)).toEqual([]);
      await library(page).getByRole("button", { name: "Delete Demo" }).click();
      await expect(dialog(page, "Delete Demo?")).toBeVisible();
      r = await new AxeBuilder({ page }).withTags(["wcag2a", "wcag2aa", "wcag21aa"]).analyze();
      expect(r.violations.map((v) => `${v.id}: ${v.nodes.map((n) => n.target.join(" ") + " " + n.failureSummary).join(", ")}`)).toEqual([]);
    });
    test("the publish review and the preview lines pass axe", async ({ app, page }) => {
      await checkedPack(app, page);
      await page.getByRole("button", { name: "Preview Demo" }).click();
      await expect(page.getByRole("list", { name: "Preview lines of Demo" })).toBeVisible();
      await library(page).getByRole("button", { name: "Publish Callum" }).click();
      const pub = page.getByRole("region", { name: "Publish" });
      await pub.getByRole("button", { name: "Prepare" }).click();
      await expect(pub.getByRole("table", { name: "Every line" })).toBeVisible();
      await page.mouse.move(0, 0);
      let r = await new AxeBuilder({ page }).withTags(["wcag2a", "wcag2aa", "wcag21aa"]).analyze();
      expect(r.violations.map((v) => `${v.id}: ${v.nodes.map((n) => n.target.join(" ") + " " + n.failureSummary).join(", ")}`)).toEqual([]);
      await pub.getByRole("button", { name: "Approve and publish" }).click();
      await expect(pub).toContainText("Released");
      r = await new AxeBuilder({ page }).withTags(["wcag2a", "wcag2aa", "wcag21aa"]).analyze();
      expect(r.violations.map((v) => `${v.id}: ${v.nodes.map((n) => n.target.join(" ") + " " + n.failureSummary).join(", ")}`)).toEqual([]);
    });
  });
}
