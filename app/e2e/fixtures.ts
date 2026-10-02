import { test as base, expect, type Page } from "@playwright/test";
import type { MockOptions, Scenario } from "../src/ipc/mock/core";
import { MOCK_BUNDLE } from "./global-setup";

export interface AppFixture {
  page: Page;
  /** Opens the app on the mock core. */
  open: (scenario?: Scenario, opts?: Omit<MockOptions, "scenario"> & { query?: string }) => Promise<void>;
  /** Commands the page sent, filtered by name. */
  calls: (cmd?: string) => Promise<{ cmd: string; args: Record<string, unknown> }[]>;
  /** The params of every call to a core method (its typed command). */
  method: (name: string) => Promise<Record<string, unknown>[]>;
  /** Runs `fn` against the mock core inside the page. */
  core: <T>(fn: string) => Promise<T>;
}

export const test = base.extend<{ app: AppFixture }>({
  app: async ({ page }, use) => {
    const app: AppFixture = {
      page,
      open: async (scenario = "library", { query = "", ...opts } = {}) => {
        await page.addInitScript((o) => (window.__QC_MOCK__ = o), { scenario, ...opts });
        await page.addInitScript({ path: MOCK_BUNDLE });
        await page.goto("/" + query);
      },
      calls: (cmd) => page.evaluate((c) => window.__qc!.calls.filter((x) => !c || x.cmd === c), cmd),
      method: async (name) => {
        const all = await page.evaluate(() => window.__qc!.calls);
        return all.filter((c) => c.cmd === name).map((c) => (c.args.params ?? c.args) as Record<string, unknown>);
      },
      core: (fn) => page.evaluate((src) => new Function("core", `return (${src})(core)`)(window.__qc!.core), fn),
    };
    await use(app);
  },
});

export { expect };

/** An open dialog by its heading. */
export const dialog = (page: Page, heading: string | RegExp) =>
  page.getByRole("dialog").filter({ has: page.getByRole("heading", { name: heading }) });

/** The grid cell around a clip card, which carries the selection. */
export const clipCell = (page: Page, name: string) => page.locator(`[aria-selected]:has(> article[aria-label="${name}"])`).first();

/** Every selected clip card or list row. */
export const selectedClips = (page: Page) => page.locator('article[aria-selected="true"], [role="gridcell"][aria-selected="true"]');

/** The list view's table. */
export const clipTable = (page: Page) => page.locator("table");
