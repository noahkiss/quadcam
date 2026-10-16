// The Samples gallery on Gear > Voices, on the mock core (design 7.4): one card per voice that
// wraps at the narrowest window, read from the core when the page opens and again after a page
// change; Play all per card and for every card, with the take playing now and Stop; Flag with
// a reason and a note on a sample or a pack line, cleared again; Re-take of the flagged takes,
// priced first; Delete per voice and Delete all, each asked first; and axe in both themes.
import AxeBuilder from "@axe-core/playwright";
import type { Page } from "@playwright/test";

/** Opens a pack row's "…" menu on the Voices page and picks one item. */
async function packMenu(page: Page, voice: string, item: string) {
  await page.getByRole("table", { name: "Voice packs on this Mac" }).getByRole("button", { name: `More actions for ${voice}` }).click();
  await page.getByRole("menu", { name: `More actions for ${voice}` }).getByRole("menuitem", { name: item }).click();
}
import { axeSchemes, dialog, expect, test, type AppFixture } from "./fixtures";

declare global {
  interface Window {
    __audio?: { src: string; paused: boolean; onended: (() => void) | null }[];
  }
}

/** An `Audio` that plays nothing and ends only when the spec says so. */
async function fakeAudio(page: Page) {
  await page.addInitScript(() => {
    window.__audio = [];
    // @ts-expect-error a stand-in for the element's constructor
    window.Audio = class {
      src: string;
      paused = false;
      onended: (() => void) | null = null;
      onerror: (() => void) | null = null;
      constructor(src: string) {
        this.src = src;
        window.__audio!.push(this);
      }
      play() {
        return Promise.resolve();
      }
      pause() {
        this.paused = true;
      }
    };
  });
}

/** Ends the take playing now. */
const endTake = (page: Page) => page.evaluate(() => window.__audio!.at(-1)!.onended?.());

async function openVoices(app: AppFixture, page: Page, seed = 0, drawer: "open" | "closed" = "open") {
  await fakeAudio(page);
  await app.open();
  if (seed) await app.core(`c => { c.gear.voice.seedSamples = ${seed}; }`);
  await page.getByRole("navigation", { name: "Library" }).getByRole("button", { name: /^Voices/ }).click();
  const samples = page.getByRole("region", { name: "Samples" });
  // The drawer starts closed; how much it hides depends on the fonts, so a test that clicks
  // inside the cards opens it first. The drawer test keeps it closed.
  if (seed && drawer === "open") {
    await expect(samples.locator("article").first()).toBeVisible();
    const more = samples.getByRole("button", { name: /^Show all / });
    if (await more.count()) await more.click();
  }
  return samples;
}

/** Samples one voice through the studio: Callum with eleven_v4, three lines. */
async function sampleCallum(page: Page) {
  const studio = page.getByRole("region", { name: "ElevenLabs" });
  await studio.getByLabel("ElevenLabs API key").fill("fake-key-0123456789abcdef");
  await studio.getByRole("button", { name: "Save key" }).click();
  await studio.getByRole("button", { name: "Sample", exact: true }).click();
  await studio.getByRole("button", { name: "Sample and pay" }).click();
  const card = page.getByRole("region", { name: "Samples" }).getByRole("article", { name: "Callum, Eleven v4" });
  await expect(card).toBeVisible();
  return card;
}

test("the gallery loads when the page opens and stays after a page change", async ({ app, page }) => {
  const samples = await openVoices(app, page, 27, "closed");
  await expect(samples.locator("article")).toHaveCount(27);
  const first = samples.getByRole("article", { name: "Aria, Eleven Turbo v2.5" });
  await expect(first).toContainText("American · en");
  await expect(first.getByRole("list", { name: /^Takes of/ }).getByRole("listitem")).toHaveCount(12);
  await page.getByRole("navigation", { name: "Library" }).getByRole("button", { name: /^Devices/ }).click();
  await expect(page.getByRole("region", { name: "Samples" })).toHaveCount(0);
  await page.getByRole("navigation", { name: "Library" }).getByRole("button", { name: /^Voices/ }).click();
  await expect(page.getByRole("region", { name: "Samples" }).locator("article")).toHaveCount(27);
  expect((await app.calls("gear_voice_samples")).length).toBeGreaterThanOrEqual(2);
});

test("the samples open as a drawer: part of the first row, then every card, remembered", async ({ app, page }) => {
  const samples = await openVoices(app, page, 27, "closed");
  const handle = samples.getByRole("button", { name: "Show all 27 cards" });
  await expect(handle).toHaveAttribute("aria-expanded", "false");
  const drawer = samples.locator('[data-component="drawer"] > div').first();
  expect((await drawer.boundingBox())!.height).toBeLessThanOrEqual(201);
  await handle.click();
  const less = samples.getByRole("button", { name: "Show less" });
  await expect(less).toHaveAttribute("aria-expanded", "true");
  expect((await drawer.boundingBox())!.height).toBeGreaterThan(600);
  // The choice stays after a page change.
  await page.getByRole("navigation", { name: "Library" }).getByRole("button", { name: /^Devices/ }).click();
  await page.getByRole("navigation", { name: "Library" }).getByRole("button", { name: /^Voices/ }).click();
  await expect(page.getByRole("region", { name: "Samples" }).getByRole("button", { name: "Show less" })).toBeVisible();
  await page.getByRole("region", { name: "Samples" }).getByRole("button", { name: "Show less" }).click();
  // Focus that moves into the hidden part opens the drawer.
  await page.getByRole("region", { name: "Samples" }).getByRole("button", { name: /^Play all of / }).last().focus();
  await expect(page.getByRole("region", { name: "Samples" }).getByRole("button", { name: "Show less" })).toBeVisible();
});

test("the cards wrap in the narrowest window, and no take runs past its card", async ({ app, page }) => {
  await page.setViewportSize({ width: 960, height: 800 });
  const samples = await openVoices(app, page, 27, "closed");
  await expect(samples.locator("article")).toHaveCount(27);
  const box = await samples.boundingBox();
  const overflow = await page.evaluate(() => {
    const out: string[] = [];
    for (const card of document.querySelectorAll('[data-component="voice-card"]')) {
      const c = card.getBoundingClientRect();
      for (const li of card.querySelectorAll("li")) {
        const r = li.getBoundingClientRect();
        if (r.right > c.right + 0.5 || r.left < c.left - 0.5) out.push(`${card.getAttribute("aria-label")}: ${li.textContent}`);
      }
      if (card.scrollWidth > card.clientWidth + 1) out.push(`${card.getAttribute("aria-label")} scrolls sideways`);
    }
    const page = document.scrollingElement!;
    if (page.scrollWidth > page.clientWidth + 1) out.push("the page scrolls sideways");
    return out;
  });
  expect(overflow).toEqual([]);
  // At least two cards share a row, and every card stays inside the section.
  const lefts = await samples.locator("article").evaluateAll((els) => els.map((e) => Math.round(e.getBoundingClientRect().left)));
  expect(new Set(lefts).size).toBeGreaterThan(1);
  const rights = await samples.locator("article").evaluateAll((els) => els.map((e) => e.getBoundingClientRect().right));
  expect(Math.max(...rights)).toBeLessThanOrEqual(box!.x + box!.width + 0.5);
});

test("Play all plays a card's takes in order with the current one marked, and Stop ends it", async ({ app, page }) => {
  const samples = await openVoices(app, page, 2);
  const card = samples.getByRole("article", { name: "Aria, Eleven Turbo v2.5" });
  const takes = card.getByRole("list", { name: /^Takes of/ }).getByRole("listitem");
  await card.getByRole("button", { name: "Play all of Aria, Eleven Turbo v2.5" }).click();
  await expect(takes.nth(0)).toHaveAttribute("aria-current", "true");
  await endTake(page);
  await expect(takes.nth(1)).toHaveAttribute("aria-current", "true");
  await expect(takes.nth(0)).not.toHaveAttribute("aria-current", "true");
  await card.getByRole("button", { name: "Stop Aria, Eleven Turbo v2.5" }).click();
  await expect(card.locator('[aria-current="true"]')).toHaveCount(0);
  expect(await page.evaluate(() => window.__audio!.at(-1)!.paused)).toBe(true);
  // Play all voices runs on into the next card after the last take of the first.
  await samples.getByRole("button", { name: "Play all voices" }).click();
  for (let i = 0; i < 12; i++) {
    await expect(takes.nth(i)).toHaveAttribute("aria-current", "true");
    await endTake(page);
  }
  const next = samples.locator("article").nth(1);
  await expect(next.locator('[aria-current="true"]')).toHaveCount(1);
  await samples.getByRole("button", { name: "Stop", exact: true }).click();
  await expect(samples.locator('[aria-current="true"]')).toHaveCount(0);
});

test("Flag marks a take with a reason and a note; Re-take asks, pays and clears it", async ({ app, page }) => {
  await openVoices(app, page);
  const card = await sampleCallum(page);
  await card.getByRole("button", { name: "Flag Armed" }).click();
  const form = page.getByRole("dialog", { name: "Flag Armed" });
  await form.getByLabel("Reason").selectOption("clipped");
  await form.getByLabel("Note").fill("the end is cut off");
  await form.getByRole("button", { name: "Save flag" }).click();
  expect((await app.method("gear_voice_flag")).at(-1)).toMatchObject({ sample: "callum-eleven-v4", pack: "", line: "SOUNDS/en/armed.wav", reason: "clipped", note: "the end is cut off", clear: false });
  await expect(card.getByRole("list", { name: "Flagged in Callum, Eleven v4" })).toContainText("Armed: Clipped: the end is cut off");
  await expect(card.getByRole("button", { name: "Change the flag on Armed" })).toBeVisible();
  // Re-take prices the flagged take first and pays only on the click.
  await card.getByRole("button", { name: "Re-take the flagged takes of Callum, Eleven v4" }).click();
  expect((await app.method("gear_voice_retake")).at(-1)).toMatchObject({ sample: "callum-eleven-v4", confirm: false });
  await expect(card).toContainText("This sends 1 line to ElevenLabs, which bills it.");
  await card.getByRole("button", { name: "Re-take and pay" }).click();
  expect((await app.method("gear_voice_retake")).at(-1)).toMatchObject({ confirm: true, digest: expect.stringMatching(/^mock-/) });
  await expect(card.getByRole("status")).toHaveText("Made 1 take again.");
  await expect(card.getByRole("list", { name: "Flagged in Callum, Eleven v4" })).toHaveCount(0);
  // A flag clears on its own too.
  await card.getByRole("button", { name: "Flag Six" }).click();
  await page.getByRole("dialog", { name: "Flag Six" }).getByRole("button", { name: "Save flag" }).click();
  await expect(card.getByRole("list", { name: "Flagged in Callum, Eleven v4" })).toContainText("Six: Mispronounced");
  await card.getByRole("button", { name: "Change the flag on Six" }).click();
  await page.getByRole("dialog", { name: "Flag Six" }).getByRole("button", { name: "Clear flag" }).click();
  await expect(card.getByRole("list", { name: "Flagged in Callum, Eleven v4" })).toHaveCount(0);
  expect((await app.method("gear_voice_flag")).at(-1)).toMatchObject({ line: "SOUNDS/en/SYSTEM/0006.wav", clear: true });
});

test("a pack line flagged in its speech check shows in the library row and on the pack's card", async ({ app, page }) => {
  const samples = await openVoices(app, page);
  await page.getByRole("button", { name: "Refresh packs" }).click();
  await page.getByRole("button", { name: "Install Demo" }).click();
  await packMenu(page, "Demo", "Check speech");
  const check = page.getByRole("dialog", { name: /^Speech check/ });
  await check.getByRole("button", { name: "Check speech" }).click();
  await check.getByRole("button", { name: "Flag Armed" }).click();
  const form = page.getByRole("dialog", { name: "Flag Armed" });
  await form.getByLabel("Reason").selectOption("noise");
  await form.getByRole("button", { name: "Save flag" }).click();
  expect((await app.method("gear_voice_flag")).at(-1)).toMatchObject({ pack: "en-demo-v1", line: "SOUNDS/en/armed.wav", reason: "noise" });
  // The flagged line needs review; an installed pack offers no re-take.
  await check.getByRole("button", { name: /^Needs review/ }).click();
  await expect(check.getByRole("table", { name: "Speech check of Demo" }).getByRole("row", { name: /^Armed/ })).toContainText("Flagged");
  await expect(check.getByRole("button", { name: "Re-take selected…" })).toHaveCount(0);
  await check.getByRole("button", { name: "Done" }).click();
  const row = page.getByRole("table", { name: "Voice packs on this Mac" }).getByRole("row", { name: /^Demo / });
  await row.getByText("1 flagged").click();
  await expect(row.getByRole("list", { name: "Re-takes and flags of Demo" })).toContainText("SOUNDS/en/armed.wav");
  await expect(row.getByRole("list", { name: "Re-takes and flags of Demo" })).toContainText("Flagged: Noise");
  const card = samples.getByRole("article", { name: "Demo, demo" });
  await expect(card.getByRole("list", { name: "Flagged in Demo, demo" })).toContainText("Armed: Noise");
  // A pack the studio did not render has no re-take.
  await expect(card.getByRole("button", { name: /^Re-take/ })).toHaveCount(0);
  await expect(card.getByRole("button", { name: /^Delete/ })).toHaveCount(0);
});

test("Delete asks first, per voice and for every sample", async ({ app, page }) => {
  const samples = await openVoices(app, page, 3);
  await expect(samples.locator("article")).toHaveCount(3);
  await samples.getByRole("button", { name: "Delete the samples of Aria, Eleven Turbo v2.5" }).click();
  await dialog(page, "Delete the samples of Aria?").getByRole("button", { name: "Cancel" }).click();
  expect(await app.calls("gear_voice_samples_delete")).toHaveLength(0);
  await samples.getByRole("button", { name: "Delete the samples of Aria, Eleven Turbo v2.5" }).click();
  await dialog(page, "Delete the samples of Aria?").getByRole("button", { name: "Delete" }).click();
  expect((await app.method("gear_voice_samples_delete")).at(-1)).toEqual({ ids: ["seed-0"], all: false });
  await expect(samples.locator("article")).toHaveCount(2);
  await samples.getByRole("button", { name: "Delete all…" }).click();
  await dialog(page, "Delete every sample?").getByRole("button", { name: "Delete" }).click();
  expect((await app.method("gear_voice_samples_delete")).at(-1)).toEqual({ ids: [], all: true });
  await expect(samples).toContainText("No samples.");
});

for (const scheme of axeSchemes) {
  test.describe(`${scheme} theme`, () => {
    test.use({ colorScheme: scheme });
    test("the Samples gallery passes axe", async ({ app, page }) => {
      const samples = await openVoices(app, page, 3);
      const card = samples.getByRole("article", { name: "Aria, Eleven Turbo v2.5" });
      await card.getByRole("button", { name: "Flag Armed" }).click();
      await page.getByRole("dialog", { name: "Flag Armed" }).getByRole("button", { name: "Save flag" }).click();
      await expect(card.getByRole("list", { name: /^Flagged in/ })).toBeVisible();
      await card.getByRole("button", { name: "Play all of Aria, Eleven Turbo v2.5" }).click();
      await card.getByRole("button", { name: "Flag Six" }).click();
      await expect(page.getByRole("dialog", { name: "Flag Six" })).toBeVisible();
      const r = await new AxeBuilder({ page }).include('[aria-label="Samples"]').withTags(["wcag2a", "wcag2aa", "wcag21aa"]).analyze();
      expect(r.violations.map((v) => `${v.id}: ${v.nodes.map((n) => n.target.join(" ") + " " + n.failureSummary).join(", ")}`)).toEqual([]);
    });
  });
}
