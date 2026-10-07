// node --test scripts/   (needs cargo and `pnpm install` in app/)
import { test } from "node:test";
import assert from "node:assert/strict";
import { allowedExpr, assets, check, chosen, crates, npmPackages, render } from "./notices.mjs";

test("license expressions", () => {
  assert.ok(allowedExpr("MIT"));
  assert.ok(allowedExpr("MIT OR Apache-2.0"));
  assert.ok(allowedExpr("MIT/Apache-2.0"));
  assert.ok(allowedExpr("(MIT OR Apache-2.0) AND Unicode-3.0"));
  assert.ok(allowedExpr("Apache-2.0 WITH LLVM-exception"));
  assert.ok(allowedExpr("MIT OR GPL-3.0-only"), "one allowed choice is enough");
  assert.ok(!allowedExpr("GPL-3.0-only"));
  assert.ok(!allowedExpr("MIT AND GPL-2.0-only"), "AND needs both");
  assert.ok(!allowedExpr("LGPL-2.1-or-later"));
  assert.ok(!allowedExpr(""));
  assert.ok(!allowedExpr("MIT OR"), "a broken expression fails");
  assert.equal(chosen("Zlib OR Apache-2.0 OR MIT"), "Zlib");
});

test("the app's inventory passes, and a planted bad license fails", () => {
  const entries = [...assets(), ...npmPackages(), ...crates()];
  assert.ok(entries.some((e) => e.kind === "crate" && e.name === "tauri"));
  assert.ok(entries.some((e) => e.kind === "npm" && e.name === "react"));
  assert.ok(entries.some((e) => e.kind === "font" && e.name === "Space Grotesk"));
  assert.ok(!entries.some((e) => e.kind === "crate" && e.name === "quadcam"), "the app itself is not a third party");
  assert.deepEqual(check(entries), []);

  const planted = [...entries, { kind: "crate", name: "planted", version: "1.0.0", license: "GPL-3.0-only", url: "", authors: [], files: [] }, { kind: "npm", name: "nolicense", version: "0.1.0", license: "", url: "", authors: [], files: [] }];
  assert.deepEqual(check(planted), ["crate planted 1.0.0: GPL-3.0-only", "npm nolicense 0.1.0: no license"]);

  const text = render(entries);
  for (const e of entries) assert.ok(text.includes(`- ${e.name}`), `${e.name} is listed`);
  assert.ok(text.includes("SIL OPEN FONT LICENSE Version 1.1"));
  assert.ok(text.includes("Copyright 2020 The Space Grotesk Project Authors"));
  assert.ok(!/<[^>\s]+@[^>\s]+>/.test(text.split("Packages that ship no license file")[1] || ""), "no author e-mail addresses in the summary");
});
