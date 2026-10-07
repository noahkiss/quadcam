#!/usr/bin/env node
// Third-party notices for what QuadCam.app ships, and the license check CI runs.
//
//   node scripts/notices.mjs --out app/dist/THIRD_PARTY_NOTICES.txt   # write the notices
//   node scripts/notices.mjs --check                                   # fail on a bad license
//
// Inventory: the Rust crates the app links (`cargo metadata`, normal dependencies reachable
// from the quadcam package on aarch64-apple-darwin), the npm packages the web view bundles
// (the `dependencies` of app/package.json and theirs), the bundled assets (fonts, icons,
// palette), and source ported from MIT projects. Each entry's license texts come from its
// own package; an entry without one names its license and repository. Needs `cargo` and
// `pnpm install` in app/.
import { execFileSync } from "node:child_process";
import { existsSync, readdirSync, readFileSync, realpathSync, statSync, writeFileSync, mkdirSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { createHash } from "node:crypto";
import { fileURLToPath } from "node:url";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");

/** Licenses an app dependency may have (SPDX ids). An expression passes when it can be met
 * with these alone: `A OR B` needs one, `A AND B` both, `A WITH exception` needs A. */
export const ALLOWED = [
  "MIT",
  "MIT-0",
  "Apache-2.0",
  "BSD-2-Clause",
  "BSD-3-Clause",
  "0BSD",
  "ISC",
  "MPL-2.0",
  "OFL-1.1",
  "Unicode-3.0",
  "Unicode-DFS-2016",
  "Zlib",
  // Permissive, no conditions beyond notice (or none): xxhash-rust and notify use them.
  "BSL-1.0",
  "CC0-1.0",
  "Unlicense",
  // Attribution only: the Solar icons.
  "CC-BY-4.0",
];

/** Splits an SPDX expression (also the old `A/B` form) into tokens. */
function tokens(expr) {
  return expr
    .replace(/\//g, " OR ")
    .replace(/\(/g, " ( ")
    .replace(/\)/g, " ) ")
    .split(/\s+/)
    .filter(Boolean);
}

/** True when the SPDX expression can be satisfied with `allowed` licenses. */
export function allowedExpr(expr, allowed = ALLOWED) {
  if (!expr || !expr.trim()) return false;
  const t = tokens(expr);
  let i = 0;
  const primary = () => {
    const x = t[i++];
    if (x === "(") {
      const v = orExpr();
      if (t[i++] !== ")") throw new Error(`unbalanced ( in ${expr}`);
      return v;
    }
    if (!x || ["AND", "OR", "WITH", ")"].includes(x)) throw new Error(`bad license expression ${expr}`);
    const ok = allowed.includes(x.replace(/\+$/, ""));
    if (t[i] === "WITH") {
      i += 2; // an exception only widens what the license allows
    }
    return ok;
  };
  const andExpr = () => {
    let v = primary();
    while (t[i] === "AND") {
      i++;
      v = primary() && v;
    }
    return v;
  };
  const orExpr = () => {
    let v = andExpr();
    while (t[i] === "OR") {
      i++;
      v = andExpr() || v;
    }
    return v;
  };
  try {
    const v = orExpr();
    return i === t.length && v;
  } catch {
    return false;
  }
}

/** The first license of an expression that the allow list takes (for a package that ships
 * no license file, the one QuadCam uses it under). */
export function chosen(expr, allowed = ALLOWED) {
  return tokens(expr).find((t) => allowed.includes(t)) || expr;
}

/** Entries whose license is missing or not allowed. */
export function check(entries, allowed = ALLOWED) {
  return entries.filter((e) => !allowedExpr(e.license, allowed)).map((e) => `${e.kind} ${e.name} ${e.version}: ${e.license || "no license"}`);
}

const LICENSE_FILE = /^(licen[cs]e|copying|notice|copyright|unlicense)([-._].*)?$/i;

/** The license files in a package folder (top level only), sorted. */
function licenseFiles(dir) {
  if (!dir || !existsSync(dir)) return [];
  return readdirSync(dir)
    .filter((f) => LICENSE_FILE.test(f) && statSync(join(dir, f)).isFile())
    .sort()
    .map((f) => ({ file: f, text: readFileSync(join(dir, f), "utf8").replace(/\r\n/g, "\n").trim() }));
}

/** Crates that compile in C code with its own license: the files to ship. hidapi's C
 *  library offers GPL-3.0, BSD or its original license; QuadCam takes the BSD one. */
const BUNDLED_C = { hidapi: ["etc/hidapi/LICENSE-bsd.txt"] };

/** Rust crates the app links, from `cargo metadata`. */
export function crates() {
  const out = execFileSync("cargo", ["metadata", "--format-version", "1", "--locked", "--filter-platform", "aarch64-apple-darwin"], {
    cwd: join(ROOT, "src-tauri"),
    maxBuffer: 1 << 28,
    encoding: "utf8",
  });
  const m = JSON.parse(out);
  const pkgs = new Map(m.packages.map((p) => [p.id, p]));
  const nodes = new Map(m.resolve.nodes.map((n) => [n.id, n]));
  const seen = new Set();
  const stack = [m.resolve.root];
  while (stack.length) {
    const id = stack.pop();
    if (seen.has(id)) continue;
    seen.add(id);
    for (const d of nodes.get(id).deps) {
      if (d.dep_kinds.some((k) => k.kind === null)) stack.push(d.pkg);
    }
  }
  seen.delete(m.resolve.root);
  return [...seen]
    .map((id) => pkgs.get(id))
    .map((p) => {
      const dir = dirname(p.manifest_path);
      const files = licenseFiles(dir);
      if (p.license_file && !files.some((f) => f.file === p.license_file)) {
        const f = join(dir, p.license_file);
        if (existsSync(f)) files.push({ file: p.license_file, text: readFileSync(f, "utf8").trim() });
      }
      // C code a crate compiles in, under the license QuadCam takes it under.
      for (const rel of BUNDLED_C[p.name] || []) {
        const f = join(dir, rel);
        if (existsSync(f)) files.push({ file: rel, text: readFileSync(f, "utf8").trim() });
      }
      return { kind: "crate", name: p.name, version: p.version, license: p.license || "", url: p.repository || p.homepage || `https://crates.io/crates/${p.name}`, authors: p.authors || [], files };
    })
    .sort((a, b) => a.name.localeCompare(b.name) || a.version.localeCompare(b.version));
}

/** The folder of package `name` as seen from `from` (Node's lookup through node_modules). */
function findPackage(name, from) {
  for (let d = from; ; d = dirname(d)) {
    const p = join(d, "node_modules", name);
    if (existsSync(join(p, "package.json"))) return realpathSync(p);
    if (dirname(d) === d) return null;
  }
}

/** npm packages the web view bundles: app/package.json `dependencies`, and theirs. */
export function npmPackages() {
  const app = join(ROOT, "app");
  const top = JSON.parse(readFileSync(join(app, "package.json"), "utf8"));
  const seen = new Map();
  const stack = Object.keys(top.dependencies || {}).map((n) => [n, app]);
  while (stack.length) {
    const [name, from] = stack.pop();
    const dir = findPackage(name, from);
    if (!dir) throw new Error(`${name} is not installed; run pnpm install in app/`);
    const pj = JSON.parse(readFileSync(join(dir, "package.json"), "utf8"));
    const key = `${pj.name}@${pj.version}`;
    if (seen.has(key)) continue;
    const license = typeof pj.license === "string" ? pj.license : pj.license?.type || (pj.licenses || []).map((l) => l.type).join(" OR ");
    const repo = typeof pj.repository === "string" ? pj.repository : pj.repository?.url;
    seen.set(key, { kind: "npm", name: pj.name, version: pj.version, license: license || "", url: (repo || pj.homepage || `https://www.npmjs.com/package/${pj.name}`).replace(/^git\+/, ""), authors: [], files: licenseFiles(dir) });
    for (const dep of Object.keys(pj.dependencies || {})) stack.push([dep, dir]);
  }
  return [...seen.values()].sort((a, b) => a.name.localeCompare(b.name) || a.version.localeCompare(b.version));
}

/** Fonts, icons and the palette bundled in the web view. */
export function assets() {
  const fonts = join(ROOT, "app/public/fonts");
  const font = (name, file, url) => ({ kind: "font", name, version: "", license: "OFL-1.1", url, authors: [], files: [{ file, text: readFileSync(join(fonts, file), "utf8").trim() }] });
  const palette = findPackage("@catppuccin/palette", join(ROOT, "app"));
  const pj = palette && JSON.parse(readFileSync(join(palette, "package.json"), "utf8"));
  return [
    font("Space Grotesk", "OFL-SpaceGrotesk.txt", "https://github.com/floriankarsten/space-grotesk"),
    font("JetBrains Mono", "OFL-JetBrainsMono.txt", "https://github.com/JetBrains/JetBrainsMono"),
    {
      kind: "icons",
      name: "Solar icon set (480 Design)",
      version: "",
      license: "CC-BY-4.0",
      url: "https://www.figma.com/community/file/1166831539721848736",
      authors: [],
      files: [{ file: "attribution", text: "Icons from the Solar icon set by 480 Design, used under the Creative Commons Attribution 4.0 International license: https://creativecommons.org/licenses/by/4.0/. Some icons were redrawn or adapted in the same style." }],
    },
    ...(pj ? [{ kind: "palette", name: "Catppuccin palette", version: pj.version, license: pj.license, url: "https://github.com/catppuccin/palette", authors: [], files: licenseFiles(palette) }] : []),
  ];
}

/** MIT source ported into QuadCam's own code, whose notice must travel with it: each has a
 * LICENSE file beside the ported source (sim-design 10.2). */
export function ported() {
  const file = "src-tauri/sim/LICENSES/propwash.txt";
  return [
    {
      kind: "ported",
      name: "propwash (parts of the simulator's physics and flight controller, ported to Rust)",
      version: "",
      license: "MIT",
      url: file,
      authors: [],
      files: [{ file: "LICENSE", text: readFileSync(join(ROOT, file), "utf8").trim() }],
    },
  ];
}

/** The notices text: a list, then each license text once with the entries that use it. */
export function render(entries) {
  const lines = [
    "QuadCam third-party notices",
    "",
    "QuadCam is MIT licensed. The app includes the software and assets below, each under",
    "its own license. Modules (ffmpeg, esptool) are not part of the app: QuadCam downloads",
    "them from their upstream when you install them, and Settings > Modules names each one's",
    "license and source.",
    "",
  ];
  const groups = [
    ["Fonts, icons and palette", (e) => ["font", "icons", "palette"].includes(e.kind)],
    ["Ported source", (e) => e.kind === "ported"],
    ["Web view (npm packages)", (e) => e.kind === "npm"],
    ["Rust crates", (e) => e.kind === "crate"],
  ];
  for (const [title, f] of groups) {
    lines.push(title, "=".repeat(title.length), "");
    for (const e of entries.filter(f)) lines.push(`- ${e.name}${e.version ? ` ${e.version}` : ""}: ${e.license} (${e.url})`);
    lines.push("");
  }
  // Each distinct text once.
  const texts = new Map();
  for (const e of entries) {
    for (const t of e.files) {
      const h = createHash("sha256").update(t.text).digest("hex");
      if (!texts.has(h)) texts.set(h, { text: t.text, users: [] });
      texts.get(h).users.push(`${e.name}${e.version ? ` ${e.version}` : ""} (${t.file})`);
    }
  }
  lines.push("License texts", "=============", "");
  for (const { text, users } of texts.values()) {
    lines.push("-".repeat(78), `Used by: ${users.join(", ")}`, "-".repeat(78), "", text, "");
  }
  const bare = entries.filter((e) => !e.files.length);
  if (bare.length) {
    const used = new Set();
    lines.push("-".repeat(78), "Packages that ship no license file. Each is used under the license named here,", "whose standard text follows; the copyright holders are the authors named.", "-".repeat(78), "");
    for (const e of bare) {
      const id = chosen(e.license);
      used.add(id);
      lines.push(`- ${e.name} ${e.version}: ${id}${e.authors.length ? `, copyright ${e.authors.map((a) => a.replace(/\s*<[^>]*>/, "")).join(", ")}` : ""} (${e.url})`);
    }
    lines.push("");
    for (const id of [...used].sort()) {
      const f = join(ROOT, "scripts/licenses", `${id}.txt`);
      lines.push("-".repeat(78), `Standard text: ${id}`, "-".repeat(78), "", existsSync(f) ? readFileSync(f, "utf8").trim() : `See https://spdx.org/licenses/${id}.html`, "");
    }
  }
  return lines.join("\n");
}

function main(argv) {
  const out = argv.includes("--out") ? argv[argv.indexOf("--out") + 1] : null;
  const entries = [...assets(), ...ported(), ...npmPackages(), ...crates()];
  const bad = check(entries);
  if (argv.includes("--check")) {
    if (bad.length) {
      console.error(`Licenses outside the allow list (scripts/notices.mjs ALLOWED):\n${bad.join("\n")}`);
      process.exit(1);
    }
    console.log(`${entries.length} entries, every license allowed.`);
  }
  if (out) {
    if (bad.length) {
      console.error(`Not writing notices; licenses outside the allow list:\n${bad.join("\n")}`);
      process.exit(1);
    }
    mkdirSync(dirname(resolve(out)), { recursive: true });
    writeFileSync(out, render(entries));
    console.log(`Wrote ${out}: ${entries.length} entries.`);
  }
  if (!out && !argv.includes("--check")) console.log(render(entries));
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) main(process.argv.slice(2));
