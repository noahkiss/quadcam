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
  expect((await app.method("gear_voice_estimate")).at(-1)).toMatchObject({ sets: ["quad", "extras", "easter"], voice: "voice-callum", model: "eleven_turbo_v2_5" });
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
  expect((await app.method("gear_voice_render")).every((p) => !p.confirm)).toBe(true);
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
  const ask = (await app.method("gear_voice_render")).at(-1) as { confirm: boolean; settings: { seed: number } };
  expect(ask.confirm).toBe(false);
  expect(ask.settings.seed).toBeGreaterThan(0);
  await studio.getByRole("button", { name: "Render and pay" }).click();
  await expect(studio).toContainText("1 batch made");
  expect((await app.method("gear_voice_render")).at(-1)).toMatchObject({ confirm: true, settings: { seed: ask.settings.seed } });
  await expect(list).toHaveCount(0);
  await expect(row.getByRole("list", { name: "Callum needs a re-take" })).toHaveCount(0);
});

test("Render pack needs one voice and one model, asks, then adds a local pack", async ({ app, page }) => {
  const studio = await openStudio(app, page);
  await saveKey(studio);
  const render = studio.getByRole("button", { name: "Render pack" });
  await studio.getByRole("table", { name: "Voices" }).getByRole("checkbox", { name: /Daniel/ }).check();
  await expect(render).toBeDisabled();
  await studio.getByRole("table", { name: "Voices" }).getByRole("checkbox", { name: /Daniel/ }).uncheck();
  await expect(render).toBeEnabled();
  await render.click();
  expect((await app.method("gear_voice_render")).at(-1)).toMatchObject({ voice: "voice-callum", model: "eleven_v4", sets: ["quad", "extras"], confirm: false });
  await expect(studio).toContainText("This sends text to ElevenLabs, which bills it.");
  await studio.getByRole("button", { name: "Render and pay" }).click();
  expect((await app.method("gear_voice_render")).at(-1)).toMatchObject({ confirm: true });
  await expect(studio).toContainText("Rendered into local-elevenlabs-voice-callum-eleven-v4");
  await expect(page.getByRole("table", { name: "Voice packs on this Mac" }).getByRole("row", { name: /^Callum local-elevenlabs-voice-callum-eleven-v4/ })).toContainText("FPV quad");
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
  await expect(studio).toContainText("Rendered into");
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
  await expect.poll(async () => (await app.method("gear_voice_estimate")).at(-1)).toMatchObject({ settings: { speed: 1.2 } });
  await studio.getByRole("button", { name: "Render pack" }).click();
  expect((await app.method("gear_voice_render")).at(-1)).toMatchObject({ settings: { speed: 1.2 }, confirm: false });
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

for (const scheme of axeSchemes) {
  test.describe(`${scheme} theme`, () => {
    test.use({ colorScheme: scheme });
    test("the Voice studio passes axe", async ({ app, page }) => {
      const studio = await openStudio(app, page);
      await saveKey(studio);
      await studio.getByRole("button", { name: "Sample", exact: true }).click();
      await studio.getByRole("button", { name: "Sample and pay" }).click();
      await expect(studio.getByRole("list", { name: "Sample voices" })).toBeVisible();
      const r = await new AxeBuilder({ page }).withTags(["wcag2a", "wcag2aa", "wcag21aa"]).analyze();
      expect(r.violations.map((v) => `${v.id}: ${v.nodes.map((n) => n.target.join(" ") + " " + n.failureSummary).join(", ")}`)).toEqual([]);
    });
  });
}
