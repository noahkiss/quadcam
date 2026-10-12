// The Voice studio on Gear > Voices, on the mock core (design 7.4): the key (kept out of every
// answer), the voice and model pickers, the credits, the line sets with a cost, a Sample that
// waits for the person's go-ahead and then shows an A/B grid, a Render into a local pack that
// joins the library, and axe.
import AxeBuilder from "@axe-core/playwright";
import type { Page } from "@playwright/test";
import { axeSchemes, expect, test, type AppFixture } from "./fixtures";

/** The value of one tile of a stat row, such as "To render" in the cost. */
const stat = (row: ReturnType<Page["getByRole"]>, label: string) => row.locator('[data-component="stat"]', { hasText: label }).locator("b");

async function openStudio(app: AppFixture, page: Page) {
  await app.open();
  await page.getByRole("navigation", { name: "Library" }).getByRole("button", { name: /^Voices/ }).click();
  return page.getByRole("region", { name: "Voice studio" });
}

/** The studio's paid render calls (not the dry runs that price the picks). */
async function renders(app: AppFixture) {
  return (await app.method("gear_voice_render_packs")).filter((p) => !p.dry_run);
}

async function saveKey(studio: ReturnType<Page["getByRole"]>) {
  await studio.getByLabel("ElevenLabs API key").fill("fake-key-0123456789abcdef");
  await studio.getByRole("button", { name: "Save key" }).click();
}

test("the key goes to the core once, is cleared from the field, and is never shown again", async ({ app, page }) => {
  const studio = await openStudio(app, page);
  await expect(studio.getByLabel("ElevenLabs API key")).toHaveAttribute("type", "password");
  await expect(studio.getByRole("button", { name: "Save key" })).toBeDisabled();
  await saveKey(studio);
  expect((await app.method("gear_voice_key")).at(-1)).toEqual({ action: "set", key: "fake-key-0123456789abcdef" });
  await expect(studio).toContainText("ElevenLabs key fake…cdef (25 chars)");
  await expect(studio.getByLabel("ElevenLabs API key")).toHaveCount(0);
  await expect(page.locator("body")).not.toContainText("0123456789");
  // Replace… opens an empty field; Keep current keeps the stored key.
  await studio.getByRole("button", { name: "Replace…" }).click();
  await expect(studio.getByLabel("ElevenLabs API key")).toHaveValue("");
  await studio.getByRole("button", { name: "Keep current" }).click();
  await expect(studio).toContainText("ElevenLabs key fake…cdef (25 chars)");
  await studio.getByRole("button", { name: "Replace…" }).click();
  await studio.getByLabel("ElevenLabs API key").fill("sk_new-REPLACEMIDDLE-9z9z");
  await studio.getByRole("button", { name: "Save key" }).click();
  await expect(studio).toContainText("ElevenLabs key sk_n…9z9z (25 chars)");
  await expect(page.locator("body")).not.toContainText("REPLACEMIDDLE");
  await expect(stat(page.getByRole("group", { name: "Summary" }), "ElevenLabs credits")).toHaveText("98,500");
  await studio.getByRole("button", { name: "Remove key" }).click();
  expect((await app.method("gear_voice_key")).at(-1)).toMatchObject({ action: "delete" });
  await expect(studio.getByLabel("ElevenLabs API key")).toBeVisible();
});

test("the pickers list the account's voices and models, and the cost follows the picks", async ({ app, page }) => {
  const studio = await openStudio(app, page);
  await saveKey(studio);
  const voices = studio.getByRole("table", { name: "Voices" });
  await expect(voices.getByRole("checkbox")).toHaveCount(8);
  await expect(voices.getByRole("checkbox", { name: /Callum/ })).toBeChecked();
  const models = studio.getByRole("table", { name: "Models" });
  await expect(models.getByRole("checkbox", { name: /\(eleven_turbo_v2_5\)/ })).toBeVisible();
  const cost = studio.getByRole("status", { name: "Cost" });
  await expect(stat(cost, "To render")).toHaveText("236");
  await expect(cost).toContainText("of 236 lines · 0 rendered");
  // The price shows in USD and in estimated credits.
  await expect(stat(cost, "Cost")).toHaveText(/^\$\d+\.\d\d$/);
  await expect(cost).toContainText("$0.08 per 1K characters");
  await expect(cost).toContainText(/[\d,]+ credits \(estimated\)/);
  await expect(models.getByRole("row", { name: /Eleven v4 eleven_v4/ })).toContainText("$0.08");
  const spent = cost.locator('[data-component="stat"]', { hasText: "Credits after" }).locator("span").last();
  const credits = async () => Number(/([\d,]+) credits/.exec((await spent.textContent()) ?? "")![1].replace(/,/g, ""));
  const full = await credits();
  // The half-price model costs half the credits for the same sets.
  await models.getByRole("checkbox", { name: /\(eleven_v4\)/ }).uncheck();
  await models.getByRole("checkbox", { name: /\(eleven_turbo_v2_5\)/ }).check();
  await expect(cost).toContainText("$0.04 per 1K characters");
  const half = await credits();
  expect(Math.abs(half - full / 2), `${half} of ${full}`).toBeLessThanOrEqual(1);
  // A second set adds its lines.
  await studio.getByRole("table", { name: "Line sets" }).getByRole("checkbox", { name: /Easter eggs/ }).check();
  await expect(cost).toContainText("of 248 lines");
  expect((await app.method("gear_voice_render_packs")).at(-1)).toMatchObject({ sets: ["quad", "extras", "easter"], voices: ["voice-callum"], models: ["eleven_turbo_v2_5"], dry_run: true });
});

test("Sample asks before it pays, then shows the A/B grid", async ({ app, page }) => {
  const studio = await openStudio(app, page);
  await saveKey(studio);
  await studio.getByRole("table", { name: "Voices" }).getByRole("checkbox", { name: /Daniel/ }).check();
  await studio.getByRole("table", { name: "Models" }).getByRole("checkbox", { name: /\(eleven_turbo_v2_5\)/ }).check();
  await studio.getByRole("button", { name: "Sample", exact: true }).click();
  expect((await app.method("gear_voice_sample")).at(-1)).toMatchObject({ voices: ["voice-callum", "voice-daniel"], models: ["eleven_v4", "eleven_turbo_v2_5"], confirm: false });
  await expect(studio).toContainText("This sends text to ElevenLabs, which bills it.");
  await expect(studio.getByRole("list", { name: "Sample voices" })).toHaveCount(0);
  const asked = (await app.method("gear_voice_sample")).length;
  await studio.getByRole("button", { name: "Sample and pay" }).click();
  await expect.poll(async () => (await app.method("gear_voice_sample")).length).toBe(asked + 1);
  // The go-ahead repeats the digest of the plan the banner priced.
  expect((await app.method("gear_voice_sample")).at(-1)).toMatchObject({ confirm: true, digest: expect.stringMatching(/^mock-/) });
  const grid = studio.getByRole("list", { name: "Sample voices" });
  // One row per line, with a numbered play button per voice and model.
  await expect(studio.getByRole("list", { name: "Voices and models" }).getByRole("listitem")).toHaveCount(4);
  await expect(grid.getByRole("listitem")).toHaveCount(3);
  await expect(grid.getByRole("listitem", { name: "Six" }).getByRole("button")).toHaveCount(4);
  await grid.getByRole("button", { name: "Play Six in Daniel with eleven_turbo_v2_5" }).click();
  // The credits went down by what the sample cost.
  await expect(stat(page.getByRole("group", { name: "Summary" }), "ElevenLabs credits")).not.toHaveText("98,500");
});

test("a picker change after Sample drops the question, so the go-ahead never pays for another plan", async ({ app, page }) => {
  const studio = await openStudio(app, page);
  await saveKey(studio);
  await studio.getByRole("button", { name: "Sample", exact: true }).click();
  await expect(studio.getByRole("button", { name: "Sample and pay" })).toBeVisible();
  await studio.getByRole("table", { name: "Voices" }).getByRole("checkbox", { name: /Daniel/ }).check();
  await expect(studio.getByRole("button", { name: "Sample and pay" })).toHaveCount(0);
  await studio.getByRole("table", { name: "Voices" }).getByRole("checkbox", { name: /Daniel/ }).uncheck();
  await studio.getByRole("button", { name: "Render pack" }).click();
  await expect(studio.getByRole("button", { name: "Render and pay" })).toBeVisible();
  await studio.getByRole("table", { name: "Line sets" }).getByRole("checkbox", { name: /Easter eggs/ }).check();
  await expect(studio.getByRole("button", { name: "Render and pay" })).toHaveCount(0);
  expect((await app.method("gear_voice_sample")).every((p) => !p.confirm)).toBe(true);
  expect((await renders(app)).every((p) => !p.confirm)).toBe(true);
});

test("a line whose cut failed a check is listed, and Re-take renders again with a new seed", async ({ app, page }) => {
  const studio = await openStudio(app, page);
  await saveKey(studio);
  await app.core(`c => { c.gear.voice.retake = "SOUNDS/en/lowbat.wav"; }`);
  await studio.getByRole("button", { name: "Render pack" }).click();
  await studio.getByRole("button", { name: "Render and pay" }).click();
  const list = studio.getByRole("list", { name: "Needs a re-take" });
  await expect(list).toContainText("Battery low (SOUNDS/en/lowbat.wav): the cut is silent");
  await expect(studio).toContainText("1 line needs a re-take and is not in the pack");
  // The pack library lists the line too (pack.json `retakes`).
  const row = page.getByRole("table", { name: "Voice packs on this Mac" }).getByRole("row", { name: /^Callum local-elevenlabs-voice-callum/ });
  await expect(row.getByRole("list", { name: "Callum needs a re-take" })).toContainText("Needs a re-take: Battery low (SOUNDS/en/lowbat.wav): the cut is silent");
  await studio.getByRole("button", { name: "Re-take" }).click();
  await expect(studio.getByRole("button", { name: "Render and pay" })).toBeVisible();
  const ask = (await renders(app)).at(-1) as { confirm: boolean; settings: { seed: number } };
  expect(ask.confirm).toBe(false);
  expect(ask.settings.seed).toBeGreaterThan(0);
  await studio.getByRole("button", { name: "Render and pay" }).click();
  await expect(studio).toContainText("1 batch made");
  expect((await renders(app)).at(-1)).toMatchObject({ confirm: true, settings: { seed: ask.settings.seed } });
  await expect(list).toHaveCount(0);
  await expect(row.getByRole("list", { name: "Callum needs a re-take" })).toHaveCount(0);
});

test("a line the speech check hears wrong is a re-take; Accept on the Voices page puts it back", async ({ app, page }) => {
  const studio = await openStudio(app, page);
  await saveKey(studio);
  await app.core(`c => { c.gear.voice.misheard = "SOUNDS/en/lowbat.wav"; }`);
  await studio.getByRole("button", { name: "Render pack" }).click();
  await studio.getByRole("button", { name: "Render and pay" }).click();
  await expect(studio.getByRole("list", { name: "Needs a re-take" })).toContainText('Battery low (SOUNDS/en/lowbat.wav): The speech check heard "Battery.".');
  const row = page.getByRole("table", { name: "Voice packs on this Mac" }).getByRole("row", { name: /^Callum local-elevenlabs-voice-callum/ });
  await expect(row).toContainText("1 heard wrong");
  await row.getByRole("button", { name: "Check speech of Callum" }).click();
  const check = page.getByRole("region", { name: "Speech check" });
  const table = check.getByRole("table", { name: "Speech check of Callum" });
  await expect(table.getByRole("row", { name: /^Battery low/ })).toContainText("Battery.");
  await expect(table.getByRole("row", { name: /^Battery low/ })).toContainText("Heard wrong");
  await expect(table.getByRole("row", { name: /^Armed/ })).toContainText("Heard right");
  await check.getByRole("button", { name: "Accept what was heard for Battery low" }).click();
  expect((await app.method("gear_voice_check")).at(-1)).toEqual({ pack: "local-elevenlabs-voice-callum-eleven-v4", lines: [], accept: ["SOUNDS/en/lowbat.wav"], install: false });
  await expect(table.getByRole("row", { name: /^Battery low/ })).toContainText("Accepted");
  await expect(row).toContainText("Heard right");
  await expect(row.getByRole("list", { name: "Callum needs a re-take" })).toHaveCount(0);
  // Check again hears the pack once more; nothing is sent to ElevenLabs.
  const paid = (await renders(app)).length;
  await check.getByRole("button", { name: "Check again" }).click();
  await expect(check.getByRole("status")).toContainText("heard wrong");
  expect((await renders(app)).length).toBe(paid);
});

test("a sample the speech check hears wrong is listed under the samples", async ({ app, page }) => {
  const studio = await openStudio(app, page);
  await saveKey(studio);
  await app.core(`c => { c.gear.voice.misheard = "SOUNDS/en/SYSTEM/0006.wav"; }`);
  await studio.getByRole("button", { name: "Sample", exact: true }).click();
  await studio.getByRole("button", { name: "Sample and pay" }).click();
  await expect(studio.getByRole("list", { name: "Heard wrong" })).toContainText("Six in Callum, eleven_v4: the speech check heard “Battery.”");
});

test("Render pack asks, then adds a local pack", async ({ app, page }) => {
  const studio = await openStudio(app, page);
  await saveKey(studio);
  await studio.getByRole("button", { name: "Render pack" }).click();
  expect((await renders(app)).at(-1)).toMatchObject({ voices: ["voice-callum"], models: ["eleven_v4"], sets: ["quad", "extras"], confirm: false });
  await expect(studio).toContainText("This sends text to ElevenLabs, which bills it.");
  await studio.getByRole("button", { name: "Render and pay" }).click();
  await expect.poll(async () => (await renders(app)).at(-1)).toMatchObject({ confirm: true });
  await expect(studio.getByRole("list", { name: "Rendered packs" })).toContainText("into local-elevenlabs-voice-callum-eleven-v4");
  await expect(page.getByRole("table", { name: "Voice packs on this Mac" }).getByRole("row", { name: /^Callum local-elevenlabs-voice-callum-eleven-v4/ })).toContainText("FPV quad");
});

test("several voices price as one plan with a row per pack, and render one pack each", async ({ app, page }) => {
  const studio = await openStudio(app, page);
  await saveKey(studio);
  const cost = studio.getByRole("status", { name: "Cost" });
  const one = Number((await stat(cost, "Characters").first().textContent())!.replace(/,/g, ""));
  await studio.getByRole("table", { name: "Voices" }).getByRole("checkbox", { name: /Daniel/ }).check();
  const byPack = studio.getByRole("table", { name: "Cost by pack" });
  await expect(byPack.getByRole("row", { name: /^Callum/ })).toBeVisible();
  await expect(byPack.getByRole("row", { name: /^Daniel/ })).toBeVisible();
  await expect(cost).toContainText("of 472 lines in 2 packs");
  await expect(stat(cost, "Characters").first()).toHaveText((2 * one).toLocaleString("en-US"));
  await studio.getByRole("button", { name: "Render packs" }).click();
  await expect(studio).toContainText("2 packs.");
  await studio.getByRole("button", { name: "Render and pay" }).click();
  const made = studio.getByRole("list", { name: "Rendered packs" });
  await expect(made.getByRole("listitem")).toHaveCount(2);
  await expect(made).toContainText("Daniel:");
  expect((await renders(app)).at(-1)).toMatchObject({ voices: ["voice-callum", "voice-daniel"], confirm: true, digest: expect.stringMatching(/^mock-/) });
});

test("a paid run shows its progress, and Stop ends it after the current batch", async ({ app, page }) => {
  const studio = await openStudio(app, page);
  await saveKey(studio);
  await studio.getByRole("table", { name: "Voices" }).getByRole("checkbox", { name: /Daniel/ }).check();
  await studio.getByRole("button", { name: "Render packs" }).click();
  await page.evaluate(() => window.__qc!.hold("gear_voice_render_packs"));
  await studio.getByRole("button", { name: "Render and pay" }).click();
  await page.evaluate(() => window.__qc!.emit("voice-progress", { kind: "render", running: true, done: 3, total: 12, voice: "voice-callum", voice_name: "Callum", model: "eleven_v4", step: "Rendering", stopping: false }));
  const progress = studio.getByRole("status", { name: "Progress" });
  await expect(progress).toContainText("Batch 3 of 12 · Callum, eleven_v4");
  await expect(progress.getByRole("progressbar", { name: "Rendering batches" })).toHaveAttribute("value", "3");
  await progress.getByRole("button", { name: "Stop" }).click();
  expect((await app.method("gear_stop")).at(-1)).toEqual({ handle: "voice-studio" });
  await page.evaluate(() => window.__qc!.emit("voice-progress", { kind: "render", running: true, done: 4, total: 12, voice: "voice-callum", voice_name: "Callum", model: "eleven_v4", step: "Rendering", stopping: true }));
  await expect(progress.getByRole("button", { name: "Stopping…" })).toBeDisabled();
  await page.evaluate(() => window.__qc!.release("gear_voice_render_packs"));
  await expect(progress).toHaveCount(0);
  await expect(studio).toContainText("Stopped after 1 of 2 packs.");
  await expect(studio.getByRole("list", { name: "Rendered packs" }).getByRole("listitem")).toHaveCount(1);
});

test("one voice and model sample as one row of play buttons", async ({ app, page }) => {
  const studio = await openStudio(app, page);
  await saveKey(studio);
  await studio.getByRole("button", { name: "Sample", exact: true }).click();
  await studio.getByRole("button", { name: "Sample and pay" }).click();
  const grid = studio.getByRole("list", { name: "Sample voices" });
  await expect(grid.getByRole("button")).toHaveCount(3);
  await expect(grid.getByRole("button", { name: "Play Six in Callum with eleven_v4" })).toHaveText("Six");
  await expect(studio.getByRole("list", { name: "Voices and models" })).toHaveCount(0);
});

test("the line sets group by aircraft, start with the quad sets, and count what is rendered", async ({ app, page }) => {
  const studio = await openStudio(app, page);
  await saveKey(studio);
  const sets = studio.getByRole("table", { name: "Line sets" });
  const groups = sets.getByRole("columnheader").filter({ hasText: /aircraft|quads/ });
  await expect(groups).toHaveText(["For your quads", "Any aircraft", "Other aircraft"]);
  const row = (t: string) => sets.getByRole("row", { name: new RegExp(`^${t} `) });
  await expect(row("FPV quad").getByRole("checkbox")).toBeChecked();
  await expect(row("FPV extras").getByRole("checkbox")).toBeChecked();
  for (const t of ["Helicopter", "Plane", "Glider", "Car", "Full EdgeTX English", "Easter eggs"]) await expect(row(t).getByRole("checkbox")).not.toBeChecked();
  await expect(row("FPV extras")).toContainText("Betaflight modes");
  const cells = (t: string) => row(t).getByRole("cell");
  await expect(cells("FPV quad").nth(1)).toHaveText("168");
  await expect(cells("FPV quad").nth(2)).toHaveText("0");
  await studio.getByRole("button", { name: "Render pack" }).click();
  await studio.getByRole("button", { name: "Render and pay" }).click();
  await expect(studio.getByRole("list", { name: "Rendered packs" })).toBeVisible();
  // The rendered sets count every line; the helicopter shares the radio's lines.
  await expect(cells("FPV quad").nth(2)).toHaveText("168");
  await expect(cells("Helicopter").nth(2)).toHaveText("140");
  const cost = studio.getByRole("status", { name: "Cost" });
  await expect(stat(cost, "To render")).toHaveText("0");
  await expect(cost).toContainText("of 236 lines · 236 rendered");
  await row("Helicopter").getByRole("checkbox").check();
  await expect(cost).toContainText("of 400 lines · 376 rendered");
  await expect(stat(cost, "To render")).toHaveText("24");
  expect((await app.method("gear_voice_sets")).at(-1)).toMatchObject({ voice: "voice-callum", model: "eleven_v4" });
  // The voice list counts each voice's lines, and so does the summary.
  await expect(studio.getByRole("table", { name: "Voices" }).getByRole("row", { name: /^Callum/ })).toContainText("236");
  await expect(stat(page.getByRole("group", { name: "Summary" }), "Lines rendered")).toHaveText("236");
  // Two voices: the set counts go, as they belong to one voice and model.
  await studio.getByRole("table", { name: "Voices" }).getByRole("checkbox", { name: "Daniel" }).check();
  await expect(cells("FPV quad").nth(2)).toHaveText("—");
});

test("Import takes… copies a folder of earlier takes, and the voice list counts them", async ({ app, page }) => {
  const studio = await openStudio(app, page);
  await saveKey(studio);
  await app.core(`c => { c.dialogAnswers.push("/Users/someone/old-takes"); }`);
  await studio.getByRole("button", { name: "Import takes…" }).click();
  await expect(studio.getByRole("status", { name: "Import" })).toHaveText("161 takes imported, 0 in the cache already, 1 left out.");
  expect((await app.method("gear_voice_import_takes")).at(-1)).toEqual({ folder: "/Users/someone/old-takes", dry_run: false });
  await studio.getByRole("table", { name: "Models" }).getByRole("checkbox", { name: /\(eleven_v4\)/ }).uncheck();
  await studio.getByRole("table", { name: "Models" }).getByRole("checkbox", { name: /\(eleven_turbo_v2_5\)/ }).check();
  await expect(studio.getByRole("table", { name: "Voices" }).getByRole("row", { name: /^Callum/ })).toContainText("161");
  await expect(stat(page.getByRole("group", { name: "Summary" }), "Lines rendered")).toHaveText("161");
  // The speed goes with every estimate and paid call.
  await studio.getByLabel("Speed").selectOption("1.2");
  await expect.poll(async () => (await app.method("gear_voice_render_packs")).at(-1)).toMatchObject({ settings: { speed: 1.2 }, dry_run: true });
  await studio.getByRole("button", { name: "Render pack" }).click();
  await expect.poll(async () => (await renders(app)).at(-1)).toMatchObject({ settings: { speed: 1.2 }, confirm: false });
});

test("a render the credits do not cover cannot start", async ({ app, page }) => {
  const studio = await openStudio(app, page);
  await saveKey(studio);
  const group = studio.getByRole("table", { name: "Line sets" });
  await group.getByRole("checkbox", { name: /Full EdgeTX English/ }).check();
  for (const t of ["FPV quad", "Helicopter", "Plane", "Glider", "FPV extras"]) await group.getByRole("checkbox", { name: t, exact: true }).check();
  await app.core(`c => { c.gear.voice.spent = 98000; }`);
  await studio.getByRole("table", { name: "Models" }).getByRole("checkbox", { name: /\(eleven_v4\)/ }).uncheck();
  await studio.getByRole("table", { name: "Models" }).getByRole("checkbox", { name: /\(eleven_v4_turbo\)/ }).check();
  await expect(studio.getByRole("alert")).toContainText("The account does not have enough credits.");
  await expect(studio.getByRole("button", { name: "Render pack" })).toBeDisabled();
});

test("the voice table shows accent and language, and filters by them", async ({ app, page }) => {
  const studio = await openStudio(app, page);
  await saveKey(studio);
  const voices = studio.getByRole("table", { name: "Voices" });
  await expect(voices.getByRole("columnheader", { name: "Accent" })).toBeVisible();
  await expect(voices.getByRole("columnheader", { name: "Language" })).toBeVisible();
  await expect(voices.getByRole("row", { name: /^Brian/ })).toContainText("en, de");
  const filters = studio.getByRole("search", { name: "Filter voices" });
  await filters.getByLabel("Accent").selectOption("british");
  await expect(voices.getByRole("checkbox")).toHaveCount(3);
  await filters.getByLabel("Language").selectOption("de");
  await expect(voices.getByRole("checkbox")).toHaveCount(0);
  await expect(studio).toContainText("No voice matches the filter.");
  await filters.getByLabel("Accent").selectOption("");
  await expect(voices.getByRole("checkbox", { name: "Brian" })).toBeVisible();
  await filters.getByLabel("Language").selectOption("");
  await filters.getByLabel("Filter").fill("lil");
  await expect(voices.getByRole("checkbox")).toHaveCount(1);
  // A voice's preview plays from the cache, fetched by the core.
  await voices.getByRole("button", { name: "Play a preview of Lily" }).click();
  expect((await app.method("gear_voice_library_preview")).at(-1)).toEqual({ url: "https://example.invalid/previews/lily.mp3" });
});

test("Browse voices searches the Voice Library, and Add asks before it changes the account", async ({ app, page }) => {
  const studio = await openStudio(app, page);
  await saveKey(studio);
  await studio.getByRole("button", { name: "Browse voices…" }).click();
  const sheet = page.getByRole("dialog", { name: "Browse voices" });
  const table = sheet.getByRole("table", { name: "Voice Library" });
  // English by default, two a page in the mock.
  expect((await app.method("gear_voice_library")).at(-1)).toMatchObject({ query: { language: "en" }, id: "" });
  await expect(table.getByRole("row", { name: /^Ranger/ })).toContainText("british");
  await expect(table.getByRole("row", { name: /^Ranger/ })).toContainText("2× credits");
  await expect(table.getByRole("row", { name: /^Ranger/ })).toContainText("30 days' notice");
  await expect(sheet.getByRole("status", { name: "Page" })).toHaveText("Page 1 · 4 voices");
  await sheet.getByRole("button", { name: "Next" }).click();
  await expect(table.getByRole("row", { name: /^Callum/ })).toContainText("In My Voices");
  await sheet.getByRole("button", { name: "Previous" }).click();
  await sheet.getByLabel("Accent").selectOption("irish");
  expect((await app.method("gear_voice_library")).at(-1)).toMatchObject({ query: { accent: "irish", language: "en", page: 0 } });
  await expect(table.getByRole("row")).toHaveCount(2);
  await table.getByRole("button", { name: "Play a preview of Maeve" }).click();
  expect((await app.method("gear_voice_library_preview")).at(-1)).toEqual({ url: "https://example.invalid/library/maeve.mp3" });
  // Add asks first: nothing reaches the account until the go-ahead.
  await table.getByRole("button", { name: "Add Maeve to My Voices" }).click();
  await expect(sheet).toContainText("This adds Maeve to your ElevenLabs account and takes one of its voice slots.");
  expect(await app.method("gear_voice_add")).toEqual([]);
  await sheet.getByRole("button", { name: "Add to My Voices" }).click();
  await expect(sheet.getByRole("status").filter({ hasText: "Added" })).toHaveText("Added Maeve to My Voices.");
  expect((await app.method("gear_voice_add")).at(-1)).toMatchObject({ voice: "lib-maeve", confirm: true });
  await expect(table.getByRole("row", { name: /^Maeve/ })).toContainText("In My Voices");
  await sheet.getByRole("button", { name: "Done" }).click();
  await expect(studio.getByRole("table", { name: "Voices" }).getByRole("checkbox", { name: "Maeve" })).toBeVisible();
});

test("Add by ID looks one voice up, and a full account says why the add failed", async ({ app, page }) => {
  const studio = await openStudio(app, page);
  await saveKey(studio);
  await app.core(`c => { c.gear.voice.slots = 8; }`);
  await studio.getByRole("button", { name: "Browse voices…" }).click();
  const sheet = page.getByRole("dialog", { name: "Browse voices" });
  await sheet.getByLabel("Voice ID").fill("lib-fjord");
  await sheet.getByRole("button", { name: "Look up" }).click();
  expect((await app.method("gear_voice_library")).at(-1)).toMatchObject({ id: "lib-fjord" });
  const table = sheet.getByRole("table", { name: "Voice Library" });
  await expect(table.getByRole("row")).toHaveCount(2);
  await expect(table.getByRole("row", { name: /^Fjord/ })).toContainText("no (norwegian)");
  await expect(sheet.getByRole("status", { name: "Page" })).toHaveCount(0);
  await table.getByRole("button", { name: "Add Fjord to My Voices" }).click();
  await sheet.getByRole("button", { name: "Add to My Voices" }).click();
  await expect(sheet.getByRole("alert")).toContainText("no free voice slot");
  await sheet.getByLabel("Voice ID").fill("nope");
  await sheet.getByRole("button", { name: "Look up" }).click();
  await expect(sheet).toContainText('Neither the account nor the Voice Library has a voice "nope".');
});

for (const scheme of axeSchemes) {
  test.describe(`${scheme} theme`, () => {
    test.use({ colorScheme: scheme });
    test("the Voice studio passes axe", async ({ app, page }) => {
      const studio = await openStudio(app, page);
      await saveKey(studio);
      await studio.getByRole("button", { name: "Sample", exact: true }).click();
      await studio.getByRole("button", { name: "Sample and pay" }).click();
      await expect(studio.getByRole("list", { name: "Sample voices" })).toBeVisible();
      // Two voices: the cost by pack, and a render going on with its progress and Stop.
      await studio.getByRole("table", { name: "Voices" }).getByRole("checkbox", { name: /Daniel/ }).check();
      await expect(studio.getByRole("table", { name: "Cost by pack" })).toBeVisible();
      await studio.getByRole("button", { name: "Render packs" }).click();
      await page.evaluate(() => window.__qc!.hold("gear_voice_render_packs"));
      await studio.getByRole("button", { name: "Render and pay" }).click();
      await page.evaluate(() => window.__qc!.emit("voice-progress", { kind: "render", running: true, done: 3, total: 12, voice: "voice-callum", voice_name: "Callum", model: "eleven_v4", step: "Rendering", stopping: false }));
      await expect(studio.getByRole("status", { name: "Progress" })).toBeVisible();
      const r = await new AxeBuilder({ page }).withTags(["wcag2a", "wcag2aa", "wcag21aa"]).analyze();
      expect(r.violations.map((v) => `${v.id}: ${v.nodes.map((n) => n.target.join(" ") + " " + n.failureSummary).join(", ")}`)).toEqual([]);
    });
  });
}

for (const scheme of axeSchemes) {
  test.describe(`${scheme} theme`, () => {
    test.use({ colorScheme: scheme });
    test("Browse voices passes axe", async ({ app, page }) => {
      const studio = await openStudio(app, page);
      await saveKey(studio);
      await studio.getByRole("button", { name: "Browse voices…" }).click();
      const sheet = page.getByRole("dialog", { name: "Browse voices" });
      await expect(sheet.getByRole("table", { name: "Voice Library" }).getByRole("row", { name: /^Ranger/ })).toBeVisible();
      await sheet.getByRole("button", { name: "Add Ranger to My Voices" }).click();
      await expect(sheet.getByRole("button", { name: "Add to My Voices" })).toBeVisible();
      const r = await new AxeBuilder({ page }).withTags(["wcag2a", "wcag2aa", "wcag21aa"]).analyze();
      expect(r.violations.map((v) => `${v.id}: ${v.nodes.map((n) => n.target.join(" ") + " " + n.failureSummary).join(", ")}`)).toEqual([]);
    });
  });
}
