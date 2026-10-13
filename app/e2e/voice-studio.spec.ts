// The ElevenLabs section on Gear > Voices, on the mock core (design 7.4): the key (kept out of every
// answer), the voice and model pickers, the credits, the line sets with a cost, a Sample that
// waits for the person's go-ahead and then fills the Samples gallery, a Render into a local pack that
// joins the library, and axe.
import AxeBuilder from "@axe-core/playwright";
import type { Page } from "@playwright/test";

/** Opens a pack row's "…" menu on the Voices page and picks one item. */
async function packMenu(page: Page, voice: string, item: string) {
  await page.getByRole("table", { name: "Voice packs on this Mac" }).getByRole("button", { name: `More actions for ${voice}` }).click();
  await page.getByRole("menu", { name: `More actions for ${voice}` }).getByRole("menuitem", { name: item }).click();
}
import { axeSchemes, expect, test, type AppFixture } from "./fixtures";

/** The value of one tile of a stat row, such as "To render" in the cost. */
const stat = (row: ReturnType<Page["getByRole"]>, label: string) => row.locator('[data-component="stat"]', { hasText: label }).locator("b");

/** Opens the app on the mock core, whose saved line sets are FPV quad and FPV extras;
 *  `before` runs against the mock core first. */
async function openStudio(app: AppFixture, page: Page, before?: string) {
  await app.open();
  if (before) await app.core(before);
  return gotoStudio(page);
}

async function gotoStudio(page: Page) {
  await page.getByRole("navigation", { name: "Library" }).getByRole("button", { name: /^Voices/ }).click();
  return page.getByRole("region", { name: "ElevenLabs" });
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
  await expect(studio).toContainText("Key fake…cdef (25 chars)");
  await expect(studio.getByLabel("ElevenLabs API key")).toHaveCount(0);
  await expect(page.locator("body")).not.toContainText("0123456789");
  // Replace… opens an empty field; Keep current keeps the stored key.
  await studio.getByRole("button", { name: "Replace…" }).click();
  await expect(studio.getByLabel("ElevenLabs API key")).toHaveValue("");
  await studio.getByRole("button", { name: "Keep current" }).click();
  await expect(studio).toContainText("Key fake…cdef (25 chars)");
  await studio.getByRole("button", { name: "Replace…" }).click();
  await studio.getByLabel("ElevenLabs API key").fill("sk_new-REPLACEMIDDLE-9z9z");
  await studio.getByRole("button", { name: "Save key" }).click();
  await expect(studio).toContainText("Key sk_n…9z9z (25 chars)");
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
  await expect(stat(cost, "To render")).toHaveText("281");
  await expect(cost).toContainText("of 281 lines · 0 rendered");
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
  await expect(cost).toContainText("of 293 lines");
  expect((await app.method("gear_voice_render_packs")).at(-1)).toMatchObject({ sets: ["quad", "extras", "quadcam", "easter"], voices: ["voice-callum"], models: ["eleven_turbo_v2_5"], dry_run: true });
});

test("Sample asks before it pays, then shows one card per voice and model in Samples", async ({ app, page }) => {
  const studio = await openStudio(app, page);
  await saveKey(studio);
  await studio.getByRole("table", { name: "Voices" }).getByRole("checkbox", { name: /Daniel/ }).check();
  await studio.getByRole("table", { name: "Models" }).getByRole("checkbox", { name: /\(eleven_turbo_v2_5\)/ }).check();
  await studio.getByRole("button", { name: "Sample", exact: true }).click();
  expect((await app.method("gear_voice_sample")).at(-1)).toMatchObject({ voices: ["voice-callum", "voice-daniel"], models: ["eleven_v4", "eleven_turbo_v2_5"], confirm: false });
  await expect(studio).toContainText("This sends text to ElevenLabs, which bills it.");
  const samples = page.getByRole("region", { name: "Samples" });
  await expect(samples.locator("article")).toHaveCount(0);
  const asked = (await app.method("gear_voice_sample")).length;
  await studio.getByRole("button", { name: "Sample and pay" }).click();
  await expect.poll(async () => (await app.method("gear_voice_sample")).length).toBe(asked + 1);
  // The go-ahead repeats the digest of the plan the banner priced.
  expect((await app.method("gear_voice_sample")).at(-1)).toMatchObject({ confirm: true, digest: expect.stringMatching(/^mock-/) });
  // One card per voice and model, each with its lines as play chips.
  await expect(samples.locator("article")).toHaveCount(4);
  const card = samples.getByRole("article", { name: "Daniel, Eleven Turbo v2.5" });
  await expect(card.getByRole("list", { name: /^Takes of/ }).getByRole("listitem")).toHaveCount(3);
  await card.getByRole("button", { name: "Play Six" }).click();
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
  const row = page.getByRole("table", { name: "Voice packs on this Mac" }).getByRole("row", { name: /^Callum / });
  await row.getByText("1 needs a re-take").click();
  const lines = row.getByRole("list", { name: "Re-takes and flags of Callum" });
  await expect(lines.getByRole("listitem")).toHaveCount(1);
  await expect(lines).toContainText("Battery low");
  await expect(lines).toContainText("the cut is silent");
  await studio.getByRole("button", { name: "Re-take" }).click();
  await expect(studio.getByRole("button", { name: "Render and pay" })).toBeVisible();
  const ask = (await renders(app)).at(-1) as { confirm: boolean; settings: { seed: number } };
  expect(ask.confirm).toBe(false);
  expect(ask.settings.seed).toBeGreaterThan(0);
  await studio.getByRole("button", { name: "Render and pay" }).click();
  await expect(studio).toContainText("1 batch made");
  expect((await renders(app)).at(-1)).toMatchObject({ confirm: true, settings: { seed: ask.settings.seed } });
  await expect(list).toHaveCount(0);
  await expect(row.getByText(/needs a re-take/)).toHaveCount(0);
});

test("a line the speech check hears wrong is a re-take; Accept on the Voices page puts it back", async ({ app, page }) => {
  const studio = await openStudio(app, page);
  await saveKey(studio);
  await app.core(`c => { c.gear.voice.misheard = "SOUNDS/en/lowbat.wav"; }`);
  await studio.getByRole("button", { name: "Render pack" }).click();
  await studio.getByRole("button", { name: "Render and pay" }).click();
  await expect(studio.getByRole("list", { name: "Needs a re-take" })).toContainText('Battery low (SOUNDS/en/lowbat.wav): The speech check heard "Battery.".');
  const row = page.getByRole("table", { name: "Voice packs on this Mac" }).getByRole("row", { name: /^Callum / });
  await expect(row).toContainText("1 heard wrong");
  await packMenu(page, "Callum", "Check speech");
  const check = page.getByRole("region", { name: "Speech check" });
  const table = check.getByRole("table", { name: "Speech check of Callum" });
  await expect(table.getByRole("row", { name: /^Battery low/ })).toContainText("Battery.");
  await expect(table.getByRole("row", { name: /^Battery low/ })).toContainText("Heard wrong");
  await expect(table.getByRole("row", { name: /^Armed/ })).toContainText("Heard right");
  await check.getByRole("button", { name: "Accept what was heard for Battery low" }).click();
  expect((await app.method("gear_voice_check")).at(-1)).toEqual({ pack: "local-elevenlabs-voice-callum-eleven-v4", lines: [], accept: ["SOUNDS/en/lowbat.wav"], install: false });
  await expect(table.getByRole("row", { name: /^Battery low/ })).toContainText("Accepted");
  await expect(row).toContainText("Heard right");
  await expect(row.getByText(/needs a re-take/)).toHaveCount(0);
  // Check again hears the pack once more; nothing is sent to ElevenLabs.
  const paid = (await renders(app)).length;
  await check.getByRole("button", { name: "Check again" }).click();
  await expect(check.getByRole("status")).toContainText("heard wrong");
  expect((await renders(app)).length).toBe(paid);
});

test("a sample the speech check hears wrong says so on its voice's card", async ({ app, page }) => {
  const studio = await openStudio(app, page);
  await saveKey(studio);
  await app.core(`c => { c.gear.voice.misheard = "SOUNDS/en/SYSTEM/0006.wav"; }`);
  await studio.getByRole("button", { name: "Sample", exact: true }).click();
  await studio.getByRole("button", { name: "Sample and pay" }).click();
  const card = page.getByRole("region", { name: "Samples" }).getByRole("article", { name: "Callum, Eleven v4" });
  await expect(card.getByRole("list", { name: "Heard wrong in Callum, Eleven v4" })).toContainText("Six: the speech check heard “Battery.”");
});

test("Render pack asks, then adds a local pack", async ({ app, page }) => {
  const studio = await openStudio(app, page);
  await saveKey(studio);
  await studio.getByRole("button", { name: "Render pack" }).click();
  expect((await renders(app)).at(-1)).toMatchObject({ voices: ["voice-callum"], models: ["eleven_v4"], sets: ["quad", "extras", "quadcam"], confirm: false });
  await expect(studio).toContainText("This sends text to ElevenLabs, which bills it.");
  await studio.getByRole("button", { name: "Render and pay" }).click();
  await expect.poll(async () => (await renders(app)).at(-1)).toMatchObject({ confirm: true });
  await expect(studio.getByRole("list", { name: "Rendered packs" })).toContainText("into local-elevenlabs-voice-callum-eleven-v4");
  await expect(page.getByRole("table", { name: "Voice packs on this Mac" }).getByRole("row", { name: /^Callum .*Eleven v4/ })).toContainText("FPV quad");
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
  await expect(cost).toContainText("of 562 lines in 2 packs");
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
  await expect(progress).toContainText("Batch 3 of 12 · Callum, Eleven v4");
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

test("the line sets sit under Full EdgeTX English, quad first, and count what is rendered", async ({ app, page }) => {
  const studio = await openStudio(app, page);
  await saveKey(studio);
  const sets = studio.getByRole("table", { name: "Line sets" });
  const order = await sets.getByRole("rowheader").allInnerTexts();
  expect(order.map((t) => t.split("\n")[0])).toEqual(["Full EdgeTX English", "FPV quad", "Telemetry scripts", "Helicopter", "Plane", "Glider", "Car", "FPV extras", "Easter eggs", "QuadCam default"]);
  const row = (t: string) => sets.getByRole("row", { name: new RegExp(`^${t} `) });
  await expect(row("FPV quad").getByRole("checkbox")).toBeChecked();
  await expect(row("FPV extras").getByRole("checkbox")).toBeChecked();
  await expect(row("QuadCam default").getByRole("checkbox")).toBeChecked();
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
  await expect(cost).toContainText("of 281 lines · 281 rendered");
  await row("Helicopter").getByRole("checkbox").check();
  await expect(cost).toContainText("of 445 lines · 421 rendered");
  await expect(stat(cost, "To render")).toHaveText("24");
  expect((await app.method("gear_voice_sets")).at(-1)).toMatchObject({ voice: "voice-callum", model: "eleven_v4" });
  // The voice list counts each voice's lines, and so does the summary.
  await expect(studio.getByRole("table", { name: "Voices" }).getByRole("row", { name: /^Callum/ })).toContainText("281");
  await expect(stat(page.getByRole("group", { name: "Summary" }), "Lines rendered")).toHaveText("281");
  // Two voices: the set counts go, as they belong to one voice and model.
  await studio.getByRole("table", { name: "Voices" }).getByRole("checkbox", { name: "Daniel" }).check();
  await expect(cells("FPV quad").nth(2)).toHaveText("—");
});

test("Full EdgeTX English ticks the sets under it, shows mixed for some, and counts shared lines once", async ({ app, page }) => {
  const studio = await openStudio(app, page);
  await saveKey(studio);
  const sets = studio.getByRole("table", { name: "Line sets" });
  const full = sets.getByRole("checkbox", { name: "Full EdgeTX English" });
  const lines = sets.getByRole("row", { name: /^Full EdgeTX English / }).getByRole("cell").nth(1);
  const mixed = () => full.evaluate((el) => (el as HTMLInputElement).indeterminate);
  // FPV quad is ticked: the parent is mixed, and counts the quad's lines inside the group.
  await expect(full).not.toBeChecked();
  expect(await mixed()).toBe(true);
  await expect(lines).toHaveText(/^168 of \d/);
  const total = Number((await lines.innerText()).split(" of ")[1].replace(",", ""));
  // Helicopter adds only its own lines: the radio's 140 are counted once.
  await sets.getByRole("checkbox", { name: "Helicopter" }).check();
  await expect(lines).toHaveText(`${168 + 164 - 140} of ${total}`);
  await full.check();
  for (const t of ["FPV quad", "Telemetry scripts", "Helicopter", "Plane", "Glider", "Car"]) await expect(sets.getByRole("checkbox", { name: t, exact: true })).toBeChecked();
  await expect(lines).toHaveText(`${total} of ${total}`);
  expect(await mixed()).toBe(false);
  // One set off: the parent is mixed again, and the rest stay ticked.
  await sets.getByRole("checkbox", { name: "Car" }).uncheck();
  await expect(full).not.toBeChecked();
  expect(await mixed()).toBe(true);
  await expect(sets.getByRole("checkbox", { name: "Plane" })).toBeChecked();
  const saved = (await app.method("gear_voice_save_sets")).at(-1)!.sets as string[];
  expect(saved).not.toContain("edgetx");
  expect(saved).not.toContain("car");
  expect(saved).toEqual(expect.arrayContaining(["quad", "heli", "plane", "glider", "scripts"]));
  // A click on the mixed box ticks everything under it, as on the Mac; a second clears it.
  await full.click();
  await expect(full).toBeChecked();
  await full.uncheck();
  for (const t of ["FPV quad", "Plane"]) await expect(sets.getByRole("checkbox", { name: t, exact: true })).not.toBeChecked();
  await expect(sets.getByRole("checkbox", { name: "FPV extras" })).toBeChecked();
});

test("a first run ticks every line set; the ticks are saved, and QuadCam default is ticked each time", async ({ app, page }) => {
  const studio = await openStudio(app, page, `c => { c.gear.voice.savedSets = null; }`);
  await saveKey(studio);
  const sets = studio.getByRole("table", { name: "Line sets" });
  const boxes = sets.getByRole("checkbox");
  await expect(boxes).toHaveCount(10);
  for (const b of await boxes.all()) await expect(b).toBeChecked();
  const note = studio.getByRole("note");
  await expect(note).toHaveCount(0);
  // Each tick saves; QuadCam default off shows the note by Render pack and still renders.
  await sets.getByRole("checkbox", { name: "Full EdgeTX English" }).uncheck();
  await sets.getByRole("checkbox", { name: "QuadCam default" }).uncheck();
  await expect(note).toHaveText("QuadCam's own cues need the QuadCam default set. Without it, they use the Mac voice.");
  await expect(studio.getByRole("button", { name: "Render pack" })).toBeEnabled();
  const saved = (await app.method("gear_voice_save_sets")).at(-1)!.sets as string[];
  expect(saved).not.toContain("edgetx");
  expect(saved).not.toContain("quadcam");
  expect(saved).toContain("easter");
  // Next time: the saved ticks come back, and QuadCam default is ticked again.
  await page.getByRole("navigation", { name: "Library" }).getByRole("button", { name: /^Devices/ }).click();
  await expect(studio).toHaveCount(0);
  const again = (await gotoStudio(page)).getByRole("table", { name: "Line sets" });
  await expect(again.getByRole("checkbox", { name: "QuadCam default" })).toBeChecked();
  await expect(again.getByRole("checkbox", { name: "Full EdgeTX English" })).not.toBeChecked();
  // Full EdgeTX English off took the sets under it off too.
  for (const t of ["FPV extras", "Easter eggs"]) await expect(again.getByRole("checkbox", { name: t, exact: true })).toBeChecked();
  for (const t of ["FPV quad", "Helicopter", "Car"]) await expect(again.getByRole("checkbox", { name: t, exact: true })).not.toBeChecked();
  await expect(page.getByRole("region", { name: "ElevenLabs" }).getByRole("note")).toHaveCount(0);
});

test("the voice list takes the full width, and the models sit below it", async ({ app, page }) => {
  const studio = await openStudio(app, page);
  await saveKey(studio);
  const voices = await studio.getByRole("table", { name: "Voices" }).boundingBox();
  const models = await studio.getByRole("table", { name: "Models" }).boundingBox();
  expect(models!.y).toBeGreaterThan(voices!.y + voices!.height - 1);
  expect(Math.abs(models!.width - voices!.width)).toBeLessThan(2);
});

test("the ElevenLabs section collapses, and stays collapsed after a page change", async ({ app, page }) => {
  const studio = await openStudio(app, page);
  const toggle = studio.getByRole("button", { name: "ElevenLabs" });
  await expect(toggle).toHaveAttribute("aria-expanded", "true");
  await toggle.click();
  await expect(toggle).toHaveAttribute("aria-expanded", "false");
  await expect(studio.getByLabel("ElevenLabs API key")).toHaveCount(0);
  await expect(page.getByRole("region", { name: "Samples" })).toHaveCount(0);
  await page.getByRole("navigation", { name: "Library" }).getByRole("button", { name: /^Devices/ }).click();
  const again = await gotoStudio(page);
  await expect(again.getByRole("button", { name: "ElevenLabs" })).toHaveAttribute("aria-expanded", "false");
  await again.getByRole("button", { name: "ElevenLabs" }).click();
  await expect(again.getByLabel("ElevenLabs API key")).toBeVisible();
});

test("the voice list scrolls inside its own box under a header that stays", async ({ app, page }) => {
  const studio = await openStudio(app, page, `c => { c.gear.voice.extraVoices = 40; }`);
  await saveKey(studio);
  const voices = studio.getByRole("table", { name: "Voices" });
  await expect(voices.getByRole("checkbox")).toHaveCount(48);
  const box = voices.locator("xpath=..");
  const size = await box.evaluate((el) => ({ client: el.clientHeight, scroll: el.scrollHeight, max: getComputedStyle(el).maxHeight, vh: window.innerHeight }));
  expect(size.client).toBeLessThanOrEqual(Math.min(0.6 * size.vh, 480) + 1);
  expect(size.scroll).toBeGreaterThan(size.client);
  // The rest of the studio is a short scroll away, not past every voice.
  const lineSets = studio.getByRole("table", { name: "Line sets" });
  const [boxBottom, setsTop] = [await box.evaluate((el) => el.getBoundingClientRect().bottom), await lineSets.evaluate((el) => el.getBoundingClientRect().top)];
  expect(setsTop - boxBottom).toBeLessThan(400);
  // Scrolled to the end: the header stays at the top of the box, and the last voice shows.
  await box.evaluate((el) => el.scrollTo(0, el.scrollHeight));
  const header = voices.getByRole("columnheader", { name: "Accent" });
  await expect(header).toBeInViewport();
  const [boxTop, headTop] = [await box.evaluate((el) => el.getBoundingClientRect().top), await header.evaluate((el) => el.getBoundingClientRect().top)];
  expect(Math.abs(headTop - boxTop)).toBeLessThanOrEqual(2);
  /** Whether a row shows inside the box, below the header. */
  const inBox = (row: RegExp) =>
    voices.getByRole("row", { name: row }).evaluate((tr) => {
      const b = tr.closest('[data-component="data-table"]')!.getBoundingClientRect();
      const h = tr.closest("table")!.querySelector("thead th")!.getBoundingClientRect();
      const r = tr.getBoundingClientRect();
      return r.top >= h.bottom - 1 && r.bottom <= b.bottom + 1;
    });
  expect(await inBox(/^Voice 40/)).toBe(true);
  expect(await inBox(/^Callum/)).toBe(false);
  // Keyboard focus brings its row into the box, clear of the header.
  await voices.getByRole("checkbox", { name: "Callum" }).focus();
  await expect.poll(() => inBox(/^Callum/)).toBe(true);
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
    test("the ElevenLabs section passes axe", async ({ app, page }) => {
      const studio = await openStudio(app, page, `c => { c.gear.voice.extraVoices = 40; }`);
      await saveKey(studio);
      await studio.getByRole("button", { name: "Sample", exact: true }).click();
      await studio.getByRole("button", { name: "Sample and pay" }).click();
      await expect(page.getByRole("region", { name: "Samples" }).locator("article")).toHaveCount(1);
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
      // The results scroll in a box of their own, like the studio's voice list.
      const box = sheet.getByRole("table", { name: "Voice Library" }).locator("xpath=..");
      expect(await box.evaluate((el) => [getComputedStyle(el).overflowY, getComputedStyle(el).maxHeight !== "none"])).toEqual(["auto", true]);
      await sheet.getByRole("button", { name: "Add Ranger to My Voices" }).click();
      await expect(sheet.getByRole("button", { name: "Add to My Voices" })).toBeVisible();
      const r = await new AxeBuilder({ page }).withTags(["wcag2a", "wcag2aa", "wcag21aa"]).analyze();
      expect(r.violations.map((v) => `${v.id}: ${v.nodes.map((n) => n.target.join(" ") + " " + n.failureSummary).join(", ")}`)).toEqual([]);
    });
  });
}
