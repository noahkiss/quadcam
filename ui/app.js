// quadcam frontend. Plain JS, no build step. The Rust core owns the session (the clips
// being imported) and the library index; this page renders both, sends the person's edits
// back, and re-renders when the core changes, including changes an agent makes over the
// control socket. trim.js holds the one trim editor the library and the import share.
"use strict";

const T = window.__TAURI__;
const invoke = T.core.invoke;
const $ = (s, root = document) => root.querySelector(s);
const $$ = (s, root = document) => [...root.querySelectorAll(s)];
const call = (method, params = null) => invoke("core_call", { method, params });

const DEFAULTS = {
  outputDir: null,
  format: "mp4",
  encoder: "videotoolbox",
  keepOriginals: false,
  addTime: false,
  defaultName: "flight",
  photosAlbum: "Drone",
  formatLabel: "DVR",
  logDir: null,
  libraryLayout: "year_day",
  placeFolders: false,
  tunables: { segment_gap_s: 5, session_gap_min: 20, tolerance_s: 30, max_log_age_days: 60 },
  // Saved places [{name, lat, lon}] and aircraft profiles; see Profile in metadata.rs.
  places: [],
  profiles: [],
  defaultProfile: "",
  recents: { keywords: [], authors: [], notes: [] },
  // How the library looks; per machine.
  libView: "grid",
  thumbSize: 3,
  libSort: { key: "date", dir: "desc" },
};

const settings = structuredClone(DEFAULTS);
let store = null;

const state = {
  tools: null,
  volumes: [],
  cards: new Map(), // mount -> card_status
  busy: false,
  session: null,
  restored: false,
  lib: null, // LibraryView from the core
  filter: { group: "all" },
  query: "",
  selected: new Set(),
  anchor: null,
  screen: "library", // library | first-run | detail
  detailId: null,
  tasks: new Map(), // task -> {done, total}
  staging: null, // {index, total, done, size, phase}
  // Import sheet
  step: "review",
  selectedClip: null,
  progress: new Map(), // clip id -> 0..1 while converting
  lastSummary: null,
  agentFormat: null,
  previewId: null,
  rTab: "details",
  dTab: "details",
};

// ---------- helpers ----------

function fmtBytes(n) {
  if (!n) return "0 B";
  const u = ["B", "KB", "MB", "GB", "TB"];
  const i = Math.min(u.length - 1, Math.floor(Math.log10(n) / 3));
  return `${(n / 10 ** (3 * i)).toFixed(i > 2 ? 1 : 0)} ${u[i]}`;
}

function fmtDur(s) {
  if (!s || s <= 0) return "0:00";
  s = Math.round(s);
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const ss = String(s % 60).padStart(2, "0");
  return h ? `${h}:${String(m).padStart(2, "0")}:${ss}` : `${m}:${ss}`;
}

function fmtLong(s) {
  const m = Math.round((s || 0) / 60);
  return m >= 60 ? `${Math.floor(m / 60)} h ${m % 60} min` : `${m} min`;
}

function fmtDay(d, long = false) {
  const dt = new Date(`${d}T12:00:00`);
  return dt.toLocaleDateString(undefined, long ? { weekday: "long", month: "short", day: "numeric", year: "numeric" } : { weekday: "short", month: "short", day: "numeric" });
}

function el(tag, attrs = {}, ...kids) {
  const e = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs)) {
    if (v === false || v == null) continue;
    if (k === "class") e.className = v;
    else if (k === "text") e.textContent = v;
    else if (k === "style") e.style.cssText = v;
    else if (k.startsWith("on")) e.addEventListener(k.slice(2), v);
    else e.setAttribute(k, v === true ? "" : v);
  }
  for (const kid of kids.flat()) if (kid != null && kid !== false) e.append(kid);
  return e;
}

function icon(name, cls) {
  const i = el("i", { "data-icon": name, class: cls });
  i.innerHTML = ICONS[name] || "";
  return i;
}

const src = (p) => (p ? T.core.convertFileSrc(p) : "");
const base = (p) => (p || "").split("/").pop();
const home = () => state.home || "";
const tilde = (p) => (p && home() && p.startsWith(home()) ? "~" + p.slice(home().length) : p || "");

let toastTimer = null;
function toast(msg, isError = false) {
  const t = $("#toast");
  t.textContent = msg;
  t.classList.toggle("error", isError);
  t.classList.add("show");
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => t.classList.remove("show"), isError ? 6000 : 3500);
}

function banner(html, kind = "warning") {
  const b = $("#banner");
  b.hidden = !html;
  b.className = kind;
  b.innerHTML = html || "";
}

// A small question with OK and Cancel, optionally with a text field. Resolves the text
// (or true), or null on cancel.
function ask(title, text, { input = null, ok = "OK", danger = false } = {}) {
  const dlg = $("#ask");
  $("#ask-title").textContent = title;
  $("#ask-text").textContent = text || "";
  $("#ask-text").hidden = !text;
  const inp = $("#ask-input");
  inp.hidden = input == null;
  inp.value = input ?? "";
  $("#ask-ok").textContent = ok;
  $("#ask-ok").className = danger ? "danger" : "primary";
  return new Promise((resolve) => {
    dlg.addEventListener("close", () => resolve(dlg.returnValue === "ok" ? (input == null ? true : inp.value) : null), { once: true });
    dlg.returnValue = "";
    dlg.showModal();
    if (input != null) { inp.focus(); inp.select(); }
  });
}

// ---------- settings ----------

async function loadSettings() {
  try {
    store = await T.store.load("settings.json", { defaults: {}, autoSave: true });
    for (const k of Object.keys(DEFAULTS)) {
      const v = await store.get(k);
      if (v !== undefined && v !== null) settings[k] = v;
    }
  } catch (e) {
    console.warn("settings store unavailable", e);
  }
}

// Tells the core, so an agent's import uses the same folder and format as the person would.
async function pushDefaults() {
  await invoke("set_defaults", {
    defaults: {
      output_dir: settings.outputDir,
      format: settings.format,
      encoder: settings.encoder,
      keep_originals: settings.keepOriginals,
      add_time: settings.addTime,
      default_name: settings.defaultName || "flight",
      photos_album: settings.photosAlbum || "",
      format_label: settings.formatLabel || "DVR",
      log_dir: settings.logDir,
      tunables: settings.tunables,
      places: settings.places,
      profiles: settings.profiles,
      default_profile: settings.defaultProfile || null,
      layout: settings.libraryLayout,
      place_folders: settings.placeFolders,
    },
  });
}

async function save(k, v) {
  settings[k] = v;
  if (store) await store.set(k, v);
  await pushDefaults();
}

// ---------- library ----------

const clipsAll = () => state.lib?.clips || [];
const libClip = (id) => clipsAll().find((c) => c.id === id);

async function loadLibrary() {
  try {
    state.lib = await call("library", {});
  } catch (e) {
    state.lib = { clips: [], unindexed: 0, totals: { clips: 0, bytes: 0, seconds: 0, flying: 0 }, groups: {}, root: settings.outputDir, exists: false };
    console.warn(e);
  }
  for (const id of [...state.selected]) if (!libClip(id)) state.selected.delete(id);
  renderAll();
}

let stripsRunning = false;
async function makeStrips() {
  if (stripsRunning || !state.tools) return;
  if (!clipsAll().some((c) => !c.strip)) return;
  stripsRunning = true;
  try {
    await call("library_strips", {});
  } catch (e) {
    console.warn(e);
  } finally {
    stripsRunning = false;
  }
}

function matches(c, f = state.filter) {
  const lastImport = state.lib?.last_import;
  switch (f.group) {
    case "last_import": if (!(c.import && c.import === lastImport)) return false; break;
    case "moments": if (!c.moments.length) return false; break;
    case "picks": if (c.flag !== "pick") return false; break;
    case "rejected": if (c.flag !== "reject") return false; break;
    case "not_in_photos": if (c.in_photos) return false; break;
    default:
  }
  if (f.day && c.date !== f.day) return false;
  if (f.before && !(c.date < f.before)) return false;
  if (f.place && (c.place || "").toLowerCase() !== f.place.toLowerCase()) return false;
  if (f.aircraft && (c.aircraft || "").toLowerCase() !== f.aircraft.toLowerCase()) return false;
  const q = state.query.trim().toLowerCase();
  if (q) {
    const hay = [c.name, c.note, c.place, c.aircraft, (c.keywords || []).join(" "), c.dvr, c.path].join(" ").toLowerCase();
    if (!q.split(/\s+/).every((w) => hay.includes(w))) return false;
  }
  return true;
}

const visible = () => sortClips(clipsAll().filter((c) => matches(c)));

// ---------- sort ----------

const SORT_FIRST_DIR = { date: "desc", rating: "desc", duration: "desc", name: "asc" };
const librarySort = () => (SORT_FIRST_DIR[settings.libSort?.key] ? settings.libSort : { key: "date", dir: "desc" });

// The one order for the grid, the list and keyboard navigation. By date: days newest
// first (or oldest first), and the clips of a day in the order they were flown.
function sortClips(list) {
  const { key, dir } = librarySort();
  const sign = dir === "asc" ? 1 : -1;
  const flown = (a, b) => (a.time || "").localeCompare(b.time || "") || a.path.localeCompare(b.path);
  const byDate = (a, b) => sign * a.date.localeCompare(b.date) || flown(a, b);
  const cmp = {
    date: byDate,
    rating: (a, b) => sign * ((a.rating || 0) - (b.rating || 0)) || byDate(a, b),
    duration: (a, b) => sign * (a.duration - b.duration) || byDate(a, b),
    name: (a, b) => sign * a.name.localeCompare(b.name, undefined, { numeric: true, sensitivity: "base" }) || byDate(a, b),
  }[key];
  return [...list].sort(cmp);
}

// A new key sorts in its natural direction; the same key again reverses it.
function setSort(key) {
  const cur = librarySort();
  save("libSort", cur.key === key ? { key, dir: cur.dir === "asc" ? "desc" : "asc" } : { key, dir: SORT_FIRST_DIR[key] });
  renderLibrary();
  syncMenu();
}

const syncMenu = () => typeof MenuBar !== "undefined" && MenuBar.sync();

function deadOf(c) {
  if (!c.keep?.length) return [];
  const out = [];
  let t = 0;
  for (const k of c.keep) {
    if (k.start - t > 0.05) out.push({ start: t, end: k.start });
    t = k.end;
  }
  if (c.duration - t > 0.05) out.push({ start: t, end: c.duration });
  return out;
}

function minibar(duration, dead) {
  const bar = el("span", { class: "minibar", "aria-hidden": "true" });
  for (const d of dead || []) bar.append(el("span", { class: "d", style: `left:${(d.start / duration) * 100}%;width:${((d.end - d.start) / duration) * 100}%` }));
  return bar;
}

function momentChips(moments) {
  const counts = new Map();
  for (const m of moments || []) if (m.kind !== "dead_air") counts.set(m.kind, (counts.get(m.kind) || 0) + 1);
  if (!counts.size) return [el("span", { class: "muted small-print", text: "No moments" })];
  return [...counts].map(([k, n]) => el("span", { class: `mchip k-${k}`, title: Trim.KIND[k] }, icon(Trim.KIND_ICON[k]), n > 1 ? String(n) : null));
}

function stars(c, interactive = true) {
  const box = el("span", { class: "stars", role: "img", "aria-label": `${c.rating || 0} of 5 stars` });
  for (let i = 1; i <= 5; i++) {
    const on = i <= (c.rating || 0);
    const s = icon("star", on ? "on" : "");
    if (interactive) {
      box.append(el("button", { type: "button", class: on ? "on" : "", "aria-label": `${i} stars`, title: `${i} stars`, onclick: (e) => { e.stopPropagation(); rate([c.id], c.rating === i ? 0 : i); } }, s));
    } else box.append(s);
  }
  return box;
}

function flagMark(c) {
  if (c.flag === "pick") return el("span", { class: "flag-pick", title: "Pick" }, icon("flag"));
  if (c.flag === "reject") return el("span", { class: "flag-reject", title: "Rejected" }, icon("close"));
  return null;
}

function stripStyle(c, tile = 2) {
  if (!c.strip) return "";
  return `background-image:url("${src(c.strip)}");background-position:${(tile / 9) * 100}% 0`;
}

// ---------- sidebar ----------

function sideItem(iconName, label, opts = {}) {
  const { count, badge, current, muted, onClick, cls } = opts;
  return el("li", {}, el("button", {
    type: "button", class: `side-item${muted ? " muted" : ""}`, "aria-current": current ? "true" : false, onclick: onClick,
  }, icon(iconName, cls), el("span", { class: "label", text: label }),
  badge ? el("span", { class: "badge new", text: badge }) : count != null ? el("span", { class: "count", text: String(count) }) : null));
}

function sideGroup(title, items) {
  return el("div", { class: "side-group" }, el("h2", { text: title }), el("ul", {}, items));
}

function setFilter(f) {
  state.filter = f;
  state.screen = "library";
  state.detailId = null;
  renderAll();
}

const same = (a, b) => JSON.stringify(a) === JSON.stringify(b);

function sessionLeft() {
  const s = state.session;
  if (!s) return 0;
  return s.plans.filter((p) => !p.skip && !s.results.some((r) => r.id === p.id && r.outcome === "verified")).length;
}

function renderSidebar() {
  const nav = $("#sidebar");
  const L = state.lib || { clips: [], groups: {} };
  const f = state.filter;
  const cur = (x) => state.screen !== "detail" && same(f, x);
  const lastN = clipsAll().filter((c) => c.last_import).length;
  const groups = [];
  groups.push(sideGroup("Library", [
    sideItem("film", "All clips", { count: L.clips.length, current: cur({ group: "all" }), onClick: () => setFilter({ group: "all" }), cls: "c-blue" }),
    lastN ? sideItem("sparkle", "Last import", { count: lastN, current: cur({ group: "last_import" }), onClick: () => setFilter({ group: "last_import" }), cls: "c-pink" }) : null,
  ]));
  const imp = [];
  const cards = state.volumes.filter((v) => v.is_card);
  for (const v of cards) {
    const st = state.cards.get(v.mount);
    const name = v.info.volume_name || base(v.mount);
    const badge = st ? (st.new ? `${st.new} new` : null) : null;
    imp.push(sideItem("sd-card", name, { badge, count: st && !st.new ? 0 : null, onClick: () => importFrom(v.mount), cls: "c-green" }));
  }
  if (!cards.length) imp.push(sideItem("sd-card", "No card inserted", { muted: true }));
  if (state.session && (sessionLeft() || !state.session.results.length)) {
    imp.push(sideItem("history", "Unfinished import", { count: state.session.clips.length, onClick: () => openImport() }));
  }
  imp.push(sideItem("folder-open", "Folder…", { onClick: () => actions["open-folder"]() }));
  for (const v of state.volumes.filter((x) => x.is_radio)) {
    imp.push(sideItem("radio", v.info.volume_name || base(v.mount), { onClick: () => useLogs(v.mount) }));
  }
  groups.push(sideGroup("Import from", imp));
  const days = L.groups?.days || [];
  if (days.length) {
    const shown = days.slice(0, 6);
    const items = shown.map(([d, n]) => sideItem("calendar", fmtDay(d), { count: n, current: cur({ group: "all", day: d }), onClick: () => setFilter({ group: "all", day: d }) }));
    if (days.length > 6) {
      const older = days.slice(6).reduce((a, [, n]) => a + n, 0);
      const before = shown[shown.length - 1][0];
      items.push(sideItem("calendar", "Older", { count: older, muted: true, current: cur({ group: "all", before }), onClick: () => setFilter({ group: "all", before }) }));
    }
    groups.push(sideGroup("Flying days", items));
  }
  const ac = L.groups?.aircraft || [];
  groups.push(sideGroup("Aircraft", ac.length
    ? ac.map(([a, n]) => sideItem("quad", a, { count: n, cls: "c-pink", current: cur({ group: "all", aircraft: a }), onClick: () => setFilter({ group: "all", aircraft: a }) }))
    : [sideItem("add", "Add aircraft", { muted: true, cls: "c-blue", onClick: () => openSettings("aircraft") })]));
  const pl = L.groups?.places || [];
  groups.push(sideGroup("Places", pl.length
    ? pl.map(([p, n]) => sideItem("map-point", p, { count: n, cls: "c-green", current: cur({ group: "all", place: p }), onClick: () => setFilter({ group: "all", place: p }) }))
    : [sideItem("add", "Add place", { muted: true, cls: "c-blue", onClick: () => openSettings("places") })]));
  if (L.clips.length) {
    const n = (fn) => clipsAll().filter(fn).length;
    groups.push(sideGroup("Smart groups", [
      sideItem("flip", "Has moments", { count: n((c) => c.moments.length), cls: "c-mauve", current: cur({ group: "moments" }), onClick: () => setFilter({ group: "moments" }) }),
      sideItem("flag", "Picks", { count: n((c) => c.flag === "pick"), cls: "c-green", current: cur({ group: "picks" }), onClick: () => setFilter({ group: "picks" }) }),
      sideItem("close", "Rejected", { count: n((c) => c.flag === "reject"), cls: "c-red", current: cur({ group: "rejected" }), onClick: () => setFilter({ group: "rejected" }) }),
      sideItem("photos", "Not in Photos", { count: n((c) => !c.in_photos), cls: "c-yellow", current: cur({ group: "not_in_photos" }), onClick: () => setFilter({ group: "not_in_photos" }) }),
    ]));
  }
  groups.push(sideFoot());
  nav.replaceChildren(...groups);
}

const TASK_LABEL = { thumbnails: "Making thumbnails", cuts: "Saving cuts", moments: "Finding dead air", rebuild: "Reading the library", stage: "Copying clips", analyse: "Checking clips", export: "Exporting" };

function sideFoot() {
  const box = el("div", { class: "side-foot" });
  const lines = [];
  for (const [task, t] of state.tasks) {
    if (t.done >= t.total) continue;
    lines.push(el("div", { class: "line" }, icon("refresh-moments", "c-blue"), el("span", { text: TASK_LABEL[task] || task }), el("span", { class: "num", text: `${Math.min(t.done + 1, t.total)} of ${t.total}` })));
    lines.push(el("span", { class: "bar" }, el("span", { style: `width:${(t.done / Math.max(1, t.total)) * 100}%` })));
  }
  const s = state.session;
  if (s?.card && s.analysed) {
    const mounted = state.volumes.find((v) => v.is_card && v.info.volume_uuid === s.card.volume_uuid);
    const copied = s.clips.length && s.clips.every((c) => c.staged && !c.stage_error);
    if (mounted && copied) {
      lines.push(el("div", { class: "line" }, icon("check-circle", "c-green"), el("span", { text: `${mounted.info.volume_name || "Card"} copied` }), el("span", { class: "ok", text: "Safe to remove" })));
    }
  }
  if (lines.length) lines.push(el("hr"));
  const L = state.lib;
  lines.push(el("div", { class: "line muted" }, el("span", { text: "Library" }), el("span", { class: "num", text: fmtBytes(L?.totals?.bytes || 0) })));
  const card = state.volumes.find((v) => v.is_card);
  const st = card && state.cards.get(card.mount);
  if (st?.free != null && st.size) lines.push(el("div", { class: "line muted" }, el("span", { text: "Card free" }), el("span", { class: "num", text: `${fmtBytes(st.free)} of ${fmtBytes(st.size)}` })));
  box.append(...lines);
  return box;
}

// ---------- library main ----------

function setScreen(screen) {
  state.screen = screen;
  if (screen !== "library") $("#sel-bar").hidden = true;
  $("#library").hidden = screen !== "library";
  $("#first-run").hidden = screen !== "first-run";
  $("#detail").hidden = screen !== "detail";
  $("#top-mid").hidden = screen === "detail";
  $$("#top-right .size, #top-right .seg, #sort").forEach((x) => (x.hidden = screen !== "library"));
}

function renderAll() {
  if (state.renaming) return;
  renderSidebar();
  const empty = !clipsAll().length && !(state.lib?.unindexed > 0);
  if (state.screen === "detail" && state.detailId && libClip(state.detailId)) {
    setScreen("detail");
    renderDetail();
  } else {
    setScreen(empty ? "first-run" : "library");
    if (empty) renderFirstRun();
    else renderLibrary();
  }
  syncMenu();
}

function renderFirstRun() {
  const prof = settings.profiles.filter((p) => p.name);
  $("#setup-aircraft").textContent = prof.length ? prof.map((p) => p.name).join(", ") : "No aircraft yet.";
  $("#setup-places").textContent = settings.places.length ? settings.places.map((p) => p.name).join(", ") : "No saved places.";
  $("#setup-folder").textContent = tilde(settings.outputDir);
  $("#setup-layout").textContent = { year_day: "A folder per year, then per flying day.", day: "A folder per flying day.", flat: "Every file in one folder." }[settings.libraryLayout];
  const logs = settings.logDir
    ? [el("span", { text: `Radio logs: ${tilde(settings.logDir)}` })]
    : [icon("radio"), el("span", { text: "Radio logs: none." }), el("button", { type: "button", class: "link", onclick: () => actions["pick-logs"]() }, "Choose the radio's LOGS folder")];
  $("#first-run-status").replaceChildren(el("span", { class: "dot-on" }), el("span", { text: "Watching for cards" }), el("span", { class: "muted", text: "·" }), ...logs);
}

function renderLibrary() {
  const content = $("#lib-content");
  content.style.setProperty("--thumb", `${[80, 104, 136, 170, 210][(settings.thumbSize || 3) - 1]}px`);
  renderBanners();
  const list = visible();
  const scroll = content.scrollTop;
  if (!list.length) {
    content.replaceChildren(el("p", { class: "empty-note", text: state.query ? "No clips match." : "No clips here." }));
  } else if (settings.libView === "list") {
    content.replaceChildren(listTable(list));
  } else if (librarySort().key !== "date") {
    content.replaceChildren(el("section", { class: "day" }, el("div", { class: "grid" }, list.map(card))));
  } else {
    const byDay = new Map();
    for (const c of list) {
      if (!byDay.has(c.date)) byDay.set(c.date, []);
      byDay.get(c.date).push(c);
    }
    const days = [];
    let first = true;
    for (const [d, cs] of byDay) {
      days.push(dayBlock(d, cs, first));
      first = false;
    }
    content.replaceChildren(...days);
  }
  content.scrollTop = scroll;
  renderFooter(list);
  renderSelBar();
  $("#sort").value = librarySort().key;
  $$("[data-action=view-grid]")[0].setAttribute("aria-pressed", String(settings.libView !== "list"));
  $$("[data-action=view-list]")[0].setAttribute("aria-pressed", String(settings.libView === "list"));
}

function renderBanners() {
  const box = $("#lib-banners");
  const out = [];
  const L = state.lib;
  if (L?.unindexed > 0) {
    out.push(el("div", { class: "lib-banner" }, icon("folder-open", "c-blue"),
      el("span", { class: "grow", text: `${L.unindexed} video${L.unindexed === 1 ? "" : "s"} in ${tilde(L.root)} ${L.unindexed === 1 ? "is" : "are"} not in the library.` }),
      el("button", { type: "button", class: "small primary", onclick: () => actions.rebuild() }, "Scan folder")));
  }
  if (state.filter.group === "rejected") {
    const n = visible().length;
    if (n) {
      out.push(el("div", { class: "lib-banner reject" }, icon("close", "c-red"),
        el("span", { class: "grow", text: `${n} rejected clip${n === 1 ? "" : "s"}.` }),
        el("button", { type: "button", class: "small danger", onclick: () => trashClips(visible().map((c) => c.id)) }, icon("trash-bin-trash"), `Move ${n} to Trash`)));
    }
  }
  box.replaceChildren(...out);
}

function dayBlock(d, cs, first) {
  const places = [...new Set(cs.map((c) => c.place).filter(Boolean))];
  const ac = [...new Set(cs.map((c) => c.aircraft).filter(Boolean))];
  const flying = cs.reduce((a, c) => a + flyingOf(c), 0);
  const sub = [...places, ...ac, `${cs.length} clip${cs.length === 1 ? "" : "s"}`, `${fmtLong(flying)} flying`].join(" · ");
  const head = el("div", { class: "day-head" },
    el("h2", { text: fmtDay(d, true) }),
    el("span", { class: "sub", text: sub }),
    cs.some((c) => c.last_import) ? el("span", { class: "chip pill-last" }, icon("sparkle"), "Last import") : null,
    el("span", { class: "act" }, el("button", { type: "button", class: "ghost small", onclick: (e) => shareClips(cs.map((c) => c.id), e.currentTarget) }, icon("share"), "Share")));
  const showSummary = first || state.filter.day === d;
  return el("section", { class: "day" }, head, showSummary ? daySummary(cs) : null, el("div", { class: "grid" }, cs.map(card)));
}

const flyingOf = (c) => (c.keep?.length ? c.keep.reduce((a, k) => a + k.end - k.start, 0) : c.duration);

function daySummary(cs) {
  const withStats = cs.filter((c) => c.stats);
  const air = withStats.length ? withStats.reduce((a, c) => a + (c.stats.armed_s || 0), 0) : cs.reduce((a, c) => a + flyingOf(c), 0);
  const packs = withStats.reduce((a, c) => a + (c.stats.packs || 0), 0);
  const volts = withStats.map((c) => c.stats.min_rx_bat_v).filter((v) => v != null);
  const best = cs.flatMap((c) => (c.moments || []).map((m) => ({ c, m }))).sort((a, b) => b.m.score - a.m.score).slice(0, 3);
  const stat = (v, l) => el("div", { class: "stat" }, el("b", { text: v }), el("span", { text: l }));
  return el("section", { class: "summary", "aria-label": "Day summary" },
    stat(String(cs.length), cs.length === 1 ? "flight" : "flights"),
    stat(fmtDur(air), withStats.length ? "armed time" : "flying"),
    packs ? stat(String(packs), packs === 1 ? "pack" : "packs") : null,
    volts.length ? stat(`${Math.min(...volts).toFixed(2)} V`, "lowest battery") : null,
    best.length ? el("div", { class: "vr" }) : null,
    best.length ? el("div", {}, el("span", { class: "lbl", text: "Best moments" }), el("ul", {}, best.map(({ c, m }) => el("li", {},
      el("button", { type: "button", class: `best k-${m.kind}`, onclick: () => openDetail(c.id, m.start) },
        el("span", { class: "mini", style: stripStyle(c, Math.min(9, Math.floor((m.start / c.duration) * 10))) }),
        icon(Trim.KIND_ICON[m.kind]), el("span", { text: c.name }), el("span", { class: "mono muted", text: fmtDur(m.start) })))))) : null);
}

function card(c) {
  const sel = state.selected.has(c.id);
  const thumb = el("div", { class: "thumb", style: stripStyle(c) },
    c.flag === "reject" ? el("span", { class: "chip tl-chip c-red" }, icon("close"), "Rejected")
      : c.cuts.length ? el("span", { class: "chip tl-chip" }, icon("scissors", "c-sky"), `${c.cuts.length} cut${c.cuts.length === 1 ? "" : "s"}`) : null,
    c.in_photos ? el("span", { class: "chip tr-chip photos-ok", title: "In Photos", "aria-label": "In Photos" }, icon("photos")) : null,
    el("span", { class: "dur", text: fmtDur(c.duration) }));
  scrubbable(thumb, c);
  const art = el("article", {
    class: `card${c.flag === "reject" ? " rejected" : ""}`, tabindex: "0", "aria-selected": String(sel), "data-id": c.id, "aria-label": c.name,
    onclick: (e) => select(c.id, e), ondblclick: () => { clearTimeout(renameTimer); openDetail(c.id); }, oncontextmenu: (e) => openMenu(e, c.id),
  }, thumb, el("div", { class: "card-body" },
    el("div", { class: "card-line" }, el("span", { class: "name", text: c.name, title: c.name, "data-name": true, onclick: (e) => nameClick(e, c.id) }),
      el("span", { class: "right" }, c.rating ? stars(c) : el("span", { class: "stars muted", text: "·····", title: "Not rated" }), flagMark(c), el("span", { class: "time", text: c.time || "" }))),
    minibar(c.duration, deadOf(c)),
    el("div", { class: "mchips" }, momentChips(c.moments))));
  return art;
}

// Hover scrub: the strip's frames follow the pointer.
function scrubbable(thumb, c) {
  if (!c.strip) return;
  let line = null;
  let label = null;
  thumb.addEventListener("mousemove", (e) => {
    const r = thumb.getBoundingClientRect();
    const f = Math.min(0.999, Math.max(0, (e.clientX - r.left) / r.width));
    thumb.style.backgroundPosition = `${(Math.floor(f * 10) / 9) * 100}% 0`;
    if (!line) {
      line = el("span", { class: "scrub" });
      label = el("span", { class: "scrub-t" });
      thumb.append(line, label);
    }
    line.style.left = label.style.left = `${f * 100}%`;
    label.textContent = fmtDur(f * c.duration);
  });
  thumb.addEventListener("mouseleave", () => {
    thumb.style.backgroundPosition = `${(2 / 9) * 100}% 0`;
    line?.remove();
    label?.remove();
    line = label = null;
  });
}

function listTable(list) {
  const rows = list.map((c) => el("tr", {
    "aria-selected": String(state.selected.has(c.id)), "data-id": c.id, tabindex: "0", class: c.flag === "reject" ? "rejected" : "",
    onclick: (e) => select(c.id, e), ondblclick: () => { clearTimeout(renameTimer); openDetail(c.id); }, oncontextmenu: (e) => openMenu(e, c.id),
  },
  el("td", {}, el("div", { class: "lthumb", style: stripStyle(c) })),
  el("td", {}, el("b", { text: c.name, "data-name": true, onclick: (e) => nameClick(e, c.id) })),
  el("td", { class: "mono", text: `${c.date} ${c.time || ""}` }),
  el("td", { class: "mono", text: fmtDur(c.duration) }),
  el("td", {}, stars(c)),
  el("td", {}, flagMark(c)),
  el("td", {}, el("div", { class: "mchips" }, momentChips(c.moments))),
  el("td", { text: c.place || "" }),
  el("td", { text: c.cuts.length ? String(c.cuts.length) : "" }),
  el("td", {}, c.in_photos ? icon("photos", "c-green") : null)));
  const { key, dir } = librarySort();
  const SORTS = { Name: "name", Date: "date", Length: "duration", Rating: "rating" };
  const th = (h) => {
    const k = SORTS[h];
    if (!k) return el("th", { text: h });
    return el("th", { "aria-sort": k === key ? (dir === "asc" ? "ascending" : "descending") : false },
      el("button", { type: "button", class: "sort", onclick: () => setSort(k) }, h, k === key ? icon(dir === "asc" ? "arrow-up" : "arrow-down", "tiny") : null));
  };
  return el("table", { class: "list-table" },
    el("thead", {}, el("tr", {}, ["", "Name", "Date", "Length", "Rating", "Flag", "Moments", "Place", "Cuts", "Photos"].map(th))),
    el("tbody", {}, rows));
}

function renderFooter(list) {
  const secs = list.reduce((a, c) => a + c.duration, 0);
  const fly = list.reduce((a, c) => a + flyingOf(c), 0);
  const bytes = list.reduce((a, c) => a + c.size + c.cuts.reduce((x, k) => x + k.size, 0), 0);
  const pct = secs ? Math.round((fly / secs) * 100) : 0;
  const bar = minibar(secs || 1, secs ? [{ start: fly, end: secs }] : []);
  $("#lib-footer").replaceChildren(
    el("span", { text: `${list.length} clip${list.length === 1 ? "" : "s"} · ${fmtLong(fly)} flying · ${fmtBytes(bytes)}` }),
    secs ? el("span", { class: "row" }, bar, `${pct} % flying`) : null,
    el("button", { type: "button", class: "link push mono", title: "Show in Finder", onclick: () => actions["reveal-library"]() }, icon("folder-open"), " ", tilde(state.lib?.root || settings.outputDir)));
}

// ---------- selection, rating, menu ----------

function select(id, e) {
  const ids = visible().map((c) => c.id);
  if (e?.metaKey) {
    state.selected.has(id) ? state.selected.delete(id) : state.selected.add(id);
  } else if (e?.shiftKey && state.anchor && ids.includes(state.anchor)) {
    const [a, b] = [ids.indexOf(state.anchor), ids.indexOf(id)].sort((x, y) => x - y);
    state.selected = new Set(ids.slice(a, b + 1));
  } else {
    state.selected = new Set([id]);
  }
  // Shift extends from the anchor; the cursor is the end that moves.
  if (!e?.shiftKey || !state.anchor) state.anchor = id;
  state.cursor = id;
  for (const node of $$("[data-id]", $("#lib-content"))) node.setAttribute("aria-selected", String(state.selected.has(node.dataset.id)));
  renderSelBar();
  syncMenu();
}

function clearSelection() {
  state.selected.clear();
  renderLibrary();
  syncMenu();
}

// The bar for two or more selected clips: rate, flag, share, add to Photos, trash.
function renderSelBar() {
  const bar = $("#sel-bar");
  const ids = selectedIds();
  bar.hidden = ids.length < 2 || state.screen !== "library";
  if (bar.hidden) return;
  const rateBox = el("span", { class: "stars", role: "group", "aria-label": "Rate" });
  for (let i = 1; i <= 5; i++) rateBox.append(el("button", { type: "button", "aria-label": `${i} stars`, title: `${i} stars`, onclick: () => rate(selectedIds(), i) }, icon("star")));
  bar.replaceChildren(
    el("b", { text: `${ids.length} selected` }),
    el("span", { class: "sep", "aria-hidden": "true" }),
    rateBox,
    el("button", { type: "button", class: "icon small", "aria-label": "Pick", title: "Pick", onclick: () => rate(selectedIds(), null, "pick") }, icon("flag")),
    el("button", { type: "button", class: "icon small", "aria-label": "Reject", title: "Reject", onclick: () => rate(selectedIds(), null, "reject") }, icon("close")),
    el("span", { class: "sep", "aria-hidden": "true" }),
    el("button", { type: "button", class: "ghost small", onclick: (e) => shareClips(selectedIds(), e.currentTarget) }, icon("share"), "Share"),
    el("button", { type: "button", class: "ghost small", onclick: () => addLibToPhotos(selectedIds()) }, icon("photos"), "Add to Photos"),
    el("button", { type: "button", class: "ghost small danger-text", onclick: () => trashClips(selectedIds()) }, icon("trash-bin-trash"), "Trash"),
    el("button", { type: "button", class: "icon small", "aria-label": "Deselect", title: "Deselect", onclick: clearSelection }, icon("close-circle")));
}

function selectAll() {
  state.selected = new Set(visible().map((c) => c.id));
  renderLibrary();
  syncMenu();
}

function setThumbSize(n) {
  n = Math.min(5, Math.max(1, n));
  settings.thumbSize = n;
  $("#thumb-size").value = n;
  save("thumbSize", n);
  renderLibrary();
  syncMenu();
}

// Previous or next clip in the library's order. In the detail view it opens that clip in
// the same tab and keeps playing if the clip was playing; in the library it moves the selection.
function stepClip(delta) {
  const list = visible().map((c) => c.id);
  if (!list.length) return;
  if (state.screen === "detail") {
    const i = list.indexOf(state.detailId);
    const next = list[i + delta];
    if (i < 0 || next == null) return;
    const v = $("#d-video");
    const playing = !v.hidden && !!v.src && !v.paused;
    openDetail(next);
    if (playing) playDetail();
    return;
  }
  const i = list.indexOf(state.cursor ?? state.anchor);
  const next = list[Math.min(list.length - 1, Math.max(0, i < 0 ? 0 : i + delta))];
  select(next);
  $(`#lib-content [data-id="${CSS.escape(next)}"]`)?.scrollIntoView({ block: "nearest" });
}

const selectedIds = () => [...state.selected].filter((id) => libClip(id));

async function rate(ids, rating, flag) {
  if (!ids.length) return;
  const before = ids.map((id) => libClip(id)).filter(Boolean).map((c) => ({ id: c.id, rating: c.rating || 0, flag: c.flag || "none" }));
  const apply = () => call("library_rate", { ids, rating: rating ?? null, flag: flag ?? null });
  try {
    await apply();
    History.push({ label: flag ? "Flag" : "Rating", redo: apply, undo: () => restoreRatings(before) });
  } catch (e) {
    toast(String(e), true);
  }
}

// Puts ratings and flags back, one core call per distinct pair.
async function restoreRatings(before) {
  const groups = new Map();
  for (const b of before) {
    const k = `${b.rating}|${b.flag}`;
    if (!groups.has(k)) groups.set(k, { rating: b.rating, flag: b.flag, ids: [] });
    groups.get(k).ids.push(b.id);
  }
  for (const g of groups.values()) await call("library_rate", { ids: g.ids, rating: g.rating, flag: g.flag });
}

async function trashClips(ids) {
  if (!ids.length) return;
  const n = ids.length;
  const ok = await ask(`Move ${n === 1 ? libClip(ids[0])?.name || "the clip" : `${n} clips`} to the Trash?`, "Cuts and kept originals go too.", { ok: "Move to Trash", danger: true });
  if (!ok) return;
  try {
    const r = await call("library_trash", { ids });
    if (r.failed.length) toast(`${r.failed.length} file${r.failed.length === 1 ? "" : "s"} did not move: ${r.failed[0][1]}`, true);
    else toast(`${r.trashed.length} file${r.trashed.length === 1 ? "" : "s"} moved to the Trash.`);
    if (state.detailId && ids.includes(state.detailId)) closeDetail();
    if (r.moved.length) {
      let moved = r.moved;
      History.push({
        label: "Move to Trash",
        undo: () => call("library_untrash", { moved }),
        redo: async () => { moved = (await call("library_trash", { ids })).moved; },
      });
    }
  } catch (e) {
    toast(String(e), true);
  }
}

// Renames a clip in place: its name on the card, list row or detail bar becomes a text
// field. Return or leaving the field saves; Escape cancels.
function startRename(id) {
  clearTimeout(renameTimer);
  const c = libClip(id);
  const scope = state.screen === "detail" ? $("#detail-bar") : $(`#lib-content [data-id="${CSS.escape(id)}"]`);
  const target = scope?.querySelector("[data-name]");
  if (!c || !target || state.renaming) return;
  state.renaming = id;
  const input = el("input", { type: "text", class: "name-edit", value: c.title || c.name, "aria-label": "Clip name", spellcheck: "false" });
  let over = false;
  const finish = async (keep) => {
    if (over) return;
    over = true;
    state.renaming = null;
    if (keep) await setClipName(id, input.value);
    renderAll();
    if (state.screen === "library") $(`#lib-content [data-id="${CSS.escape(id)}"]`)?.focus();
  };
  input.addEventListener("keydown", (e) => {
    if (e.key === "Enter") { e.preventDefault(); finish(true); }
    if (e.key === "Escape") { e.preventDefault(); e.stopPropagation(); finish(false); }
  });
  input.addEventListener("blur", () => finish(true));
  for (const ev of ["click", "dblclick", "mousedown"]) input.addEventListener(ev, (e) => e.stopPropagation());
  target.replaceChildren(input);
  input.focus();
  input.select();
}

// A click on the name of the one selected clip renames it, unless a double-click follows.
let renameTimer = null;
function nameClick(e, id) {
  if (state.selected.size !== 1 || !state.selected.has(id) || e.metaKey || e.shiftKey) return;
  e.stopPropagation();
  clearTimeout(renameTimer);
  renameTimer = setTimeout(() => startRename(id), 450);
}

async function setClipName(id, name) {
  const c = libClip(id);
  name = name.trim();
  if (!c || !name || name === (c.title || c.name)) return;
  const old = c.title || c.name;
  try {
    await call("library_rename", { id, name });
    History.push({ label: "Rename", undo: () => call("library_rename", { id, name: old }), redo: () => call("library_rename", { id, name }) });
  } catch (e) {
    toast(String(e), true);
  }
}

async function addLibToPhotos(ids) {
  try {
    const r = await call("library_photos", { ids, album: settings.photosAlbum ?? "" });
    const where = r.album ? `the ${r.album} album` : "Photos";
    if (r.failed.length) toast(`${r.added.length} added to ${where}; ${r.failed.length} failed: ${r.failed[0][1]}`, true);
    else toast(`${r.added.length} file${r.added.length === 1 ? "" : "s"} added to ${where}.`);
  } catch (e) {
    toast(String(e), true);
  }
}

// The macOS Share menu for clips and their cuts, shown next to `anchor` (an element), else
// next to the clip's card or the detail bar's Share button.
async function shareClips(ids, anchor) {
  const root = state.lib?.root || "";
  const files = ids.map(libClip).filter(Boolean).flatMap((c) => [c.file, ...c.cuts.map((k) => `${root}/${k.path}`)]);
  if (!files.length) return;
  anchor = anchor || (state.screen === "detail" ? $("#detail-bar [data-share]") : menuAnchor(ids[0]));
  const r = anchor?.getBoundingClientRect() || { left: innerWidth / 2, top: 60, width: 1, height: 1 };
  try {
    await invoke("share", { paths: files, x: r.left, y: r.top, w: r.width, h: r.height });
  } catch (e) {
    toast(String(e), true);
  }
}

const menuAnchor = (id) => $(`#lib-content [data-id="${CSS.escape(id)}"]`);

async function rescan(id) {
  try {
    await call("library_rescan", { id });
    toast("Dead air found again.");
  } catch (e) {
    toast(String(e), true);
  }
}

function openMenu(e, id) {
  e.preventDefault();
  if (!state.selected.has(id)) select(id);
  const ids = selectedIds();
  const many = ids.length > 1;
  const c = libClip(id);
  const item = (ic, label, key, fn, cls) => el("li", {}, el("button", { type: "button", role: "menuitem", class: cls, onclick: () => { closeMenu(); fn(); } }, icon(ic), el("span", { text: label }), el("span", { class: "key", text: key })));
  const sep = () => el("li", { role: "separator" });
  const menu = $("#menu");
  menu.replaceChildren(
    many ? null : item("pen", "Rename", "⏎", () => startRename(id)),
    many ? null : item("tag", "Edit details", "⌘I", () => openDetail(id, null, "details")),
    many ? null : item("scissors", "Trim and cuts", "T", () => openDetail(id)),
    sep(),
    item("share", "Share…", "", () => shareClips(ids, menuAnchor(id))),
    item("photos", many ? `Add ${ids.length} to Photos` : "Add to Photos", "", () => addLibToPhotos(ids)),
    many ? null : item("finder", "Show in Finder", "⌘R", () => T.opener.revealItemInDir(c.file)),
    many ? null : item("refresh-moments", "Find dead air again", "", () => rescan(id)),
    sep(),
    item("trash-bin-trash", many ? `Move ${ids.length} to Trash` : "Move to Trash", "⌘⌫", () => trashClips(ids), "danger"),
  );
  menu.hidden = false;
  const r = menu.getBoundingClientRect();
  menu.style.left = `${Math.min(e.clientX, innerWidth - r.width - 8)}px`;
  menu.style.top = `${Math.min(e.clientY, innerHeight - r.height - 8)}px`;
  menu.querySelector("button")?.focus();
}

function closeMenu() {
  $("#menu").hidden = true;
}

document.addEventListener("click", (e) => {
  if (!e.target.closest("#menu")) closeMenu();
});

// ---------- clip detail ----------

let dTrim = null;

function openDetail(id, at = null, tab = null) {
  state.detailId = id;
  state.screen = "detail";
  if (tab) state.dTab = tab;
  state.selected = new Set([id]);
  state.detailSeek = at;
  state.detailVideo = null;
  const v = $("#d-video");
  v.pause();
  v.removeAttribute("src");
  v.hidden = true;
  $("[data-action=d-play]").hidden = false;
  renderAll();
  if (at != null) playDetail();
}

function closeDetail() {
  $("#d-video").pause();
  state.detailId = null;
  state.screen = "library";
  renderAll();
}

function libTrimModel(c) {
  const cuts = [
    ...c.cuts.map((k) => ({ start: k.start, end: k.end, state: "saved", file: k.path })),
    ...(c.pending_cuts || []).map((k) => ({ start: k.start, end: k.end, state: "new" })),
  ].sort((a, b) => a.start - b.start);
  return { key: `lib:${c.id}`, duration: c.duration, moments: c.moments, deadAir: deadOf(c), keep: c.keep, cuts, hasLog: false, logOffset: 0 };
}

function renderDetail() {
  const c = libClip(state.detailId);
  if (!c) return closeDetail();
  const bar = $("#detail-bar");
  bar.replaceChildren(
    el("div", { class: "crumbs" },
      el("button", { type: "button", class: "ghost small", onclick: closeDetail }, icon("arrow-left"), "Library"),
      el("span", { class: "sep", text: "/" }), el("span", { class: "muted", text: fmtDay(c.date) }), el("span", { class: "sep", text: "/" }),
      el("b", { class: "name", text: c.name, title: "Rename", "data-name": true, onclick: () => startRename(c.id) }),
      c.aircraft ? el("span", { class: "chip" }, icon("quad", "c-pink"), c.aircraft) : null,
      c.place ? el("span", { class: "chip" }, icon("map-point", "c-green"), c.place) : null),
    el("div", { class: "right" },
      stars(c),
      el("button", { type: "button", class: `icon small ${c.flag === "pick" ? "flag-pick" : ""}`, "aria-label": "Pick", title: "Pick (P)", "aria-pressed": String(c.flag === "pick"), onclick: () => rate([c.id], null, c.flag === "pick" ? "none" : "pick") }, icon("flag")),
      el("button", { type: "button", class: `icon small ${c.flag === "reject" ? "flag-reject" : ""}`, "aria-label": "Reject", title: "Reject (X)", "aria-pressed": String(c.flag === "reject"), onclick: () => rate([c.id], null, c.flag === "reject" ? "none" : "reject") }, icon("close")),
      c.in_photos ? el("span", { class: "chip photos-in", title: "In Photos" }, icon("photos"), "In Photos") : null,
      el("button", { type: "button", class: "ghost small", "data-share": true, onclick: (e) => shareClips([c.id], e.currentTarget) }, icon("share"), "Share"),
      el("button", { type: "button", class: "icon small", "aria-label": "Show in Finder", title: "Show in Finder", onclick: () => T.opener.revealItemInDir(c.file) }, icon("finder")),
      el("button", { type: "button", class: "icon small", "aria-label": "More actions", onclick: (e) => openMenu(e, c.id) }, icon("dots"))));
  // The poster is one frame of the strip.
  $("#d-img").hidden = true;
  $("#d-player").style.cssText = $("#d-video").hidden && c.strip ? `${stripStyle(c)};background-size:1000% 100%` : "";
  if (!dTrim) {
    dTrim = new Trim.Editor($("#d-trim"), {
      toast,
      momentsEl: $("#d-moments"),
      setCuts: async (cuts) => {
        const id = state.detailId;
        try {
          const r = await Trim.apply((k, removed) => call("library_cuts", { id, cuts: k, removed_cuts: removed }), cuts);
          if (r?.status === "applied" && (r.trashed?.length || r.kept?.length)) toast(r.trashed?.length ? "Cut file moved to the Trash." : "Cut file kept as its own clip.");
        } catch (e) {
          toast(String(e), true);
        }
      },
      setOffset: async () => {},
      save: async () => {
        try {
          const made = await call("library_export_cuts", { id: state.detailId });
          toast(`${made.length} cut${made.length === 1 ? "" : "s"} saved.`);
        } catch (e) {
          toast(String(e), true);
        }
      },
    });
    dTrim.attachVideo($("#d-video"));
  }
  dTrim.render(libTrimModel(c));
  $$("#d-tabs .tab").forEach((t) => { const on = t.dataset.tab === state.dTab; t.classList.toggle("on", on); t.setAttribute("aria-selected", String(on)); });
  $("#d-details").hidden = state.dTab !== "details";
  $("#d-flight").hidden = state.dTab !== "flight";
  renderLibDetails(c);
  renderFlight($("#d-flight"), c.stats, c.moments, c.path);
}

function renderLibDetails(c) {
  const box = $("#d-details");
  if (box.contains(document.activeElement) && box.dataset.id === c.id) return;
  box.dataset.id = c.id;
  const placeSel = el("select", { "aria-label": "Place" },
    el("option", { value: "", text: "None" }),
    ...settings.places.map((p) => el("option", { value: p.name, text: p.name, selected: (c.place || "").toLowerCase() === p.name.toLowerCase() })),
    c.place && !settings.places.some((p) => p.name.toLowerCase() === c.place.toLowerCase()) ? el("option", { value: c.place, text: c.place, selected: true }) : null);
  placeSel.addEventListener("change", () => libEdit(c.id, { place: placeSel.value }));
  const kwBox = el("div", { class: "kw" });
  const renderKw = (words) => {
    const input = el("input", { type: "text", "aria-label": "Add keyword", placeholder: "Add…", list: "recent-keywords" });
    input.addEventListener("keydown", (e) => {
      if (e.key === "Enter" && input.value.trim()) {
        e.preventDefault();
        remember("keywords", input.value);
        libEdit(c.id, { keywords: [...words, input.value.trim()] });
      }
    });
    kwBox.replaceChildren(...words.map((w, i) => el("span", { class: "chip" }, w, el("button", { type: "button", "aria-label": `Remove ${w}`, onclick: () => libEdit(c.id, { keywords: words.filter((_, j) => j !== i) }) }, icon("close")))), input);
  };
  renderKw(c.keywords || []);
  const author = el("input", { type: "text", value: c.author || "", list: "recent-authors", spellcheck: "false" });
  author.addEventListener("change", () => { remember("authors", author.value); libEdit(c.id, { author: author.value }); });
  const note = el("input", { type: "text", value: c.note || "", list: "recent-notes", spellcheck: "false" });
  note.addEventListener("change", () => { remember("notes", note.value); libEdit(c.id, { note: note.value }); });
  box.replaceChildren(
    el("label", { class: "field" }, el("span", { text: "Aircraft" }), el("input", { type: "text", value: c.aircraft || "", disabled: true })),
    el("label", { class: "field" }, el("span", { text: "Place" }), placeSel, c.location ? el("span", { class: "hint mono", text: `${c.location.lat.toFixed(4)}, ${c.location.lon.toFixed(4)}` }) : null),
    el("div", { class: "field" }, el("span", { text: "Keywords" }), kwBox),
    el("label", { class: "field" }, el("span", { text: "Author" }), author),
    el("label", { class: "field" }, el("span", { text: "Note" }), note),
    el("dl", { class: "meta" },
      el("dt", { text: "File" }), el("dd", { text: c.path }),
      el("dt", { text: "Size" }), el("dd", { text: fmtBytes(c.size) }),
      c.dvr ? [el("dt", { text: "DVR file" }), el("dd", { text: c.dvr })] : null,
      c.original ? [el("dt", { text: "Original" }), el("dd", { text: c.original })] : null));
}

async function libEdit(id, patch) {
  const c = libClip(id);
  const before = {};
  if (c) {
    if ("note" in patch) before.note = c.note || "";
    if ("keywords" in patch) before.keywords = c.keywords || [];
    if ("author" in patch) before.author = c.author || "";
    if ("place" in patch || "location" in patch) {
      if (c.location) before.location = { ...c.location, name: c.place || c.location.name || null };
      else before.place = "";
    }
  }
  try {
    await call("library_edit", { id, ...patch });
    if (c) History.push({ label: "Edit", undo: () => call("library_edit", { id, ...before }), redo: () => call("library_edit", { id, ...patch }) });
  } catch (e) {
    toast(String(e), true);
  }
}

function renderFlight(box, f, moments, file) {
  if (!f) {
    box.replaceChildren(el("p", { class: "muted", text: "No radio log for this clip." }));
    return;
  }
  const s = Math.round(f.armed_s || 0);
  const kinds = new Map();
  for (const m of moments || []) kinds.set(m.kind, (kinds.get(m.kind) || 0) + 1);
  const box2 = (label, value, sub) => el("div", { class: "stat-box" }, el("span", { class: "lbl", text: label }), el("b", { text: value }), el("span", { text: sub || "" }));
  box.replaceChildren(el("div", { class: "stats" },
    box2("Armed time", fmtDur(s), `${f.packs} pack${f.packs === 1 ? "" : "s"}`),
    box2("Lowest battery", f.min_rx_bat_v != null ? `${f.min_rx_bat_v.toFixed(2)} V` : "–", "receiver voltage"),
    box2("Link quality", f.min_lq != null ? `${Math.round(f.min_lq)} %` : "–", "lowest"),
    box2("Signal", f.min_rssi_db != null ? `${Math.round(f.min_rssi_db)} dBm` : "–", "weakest RSSI"),
    box2("Max throttle", f.max_throttle != null ? `${Math.round(f.max_throttle * 100)} %` : "–", ""),
    box2("Moments", String((moments || []).length), [...kinds].map(([k, n]) => (n > 1 ? `${Trim.KIND[k].toLowerCase()} ×${n}` : Trim.KIND[k].toLowerCase())).join(" · "))),
  );
}

async function playDetail() {
  const c = libClip(state.detailId);
  if (!c) return;
  const play = $("[data-action=d-play]");
  play.disabled = true;
  play.querySelector("span").textContent = "Loading…";
  try {
    const path = await call("library_preview", { id: c.id });
    if (state.detailId !== c.id) return;
    const v = $("#d-video");
    v.src = src(path);
    v.hidden = false;
    v.controls = true;
    $("#d-player").style.cssText = "";
    play.hidden = true;
    if (state.detailSeek != null) {
      v.addEventListener("loadedmetadata", () => { v.currentTime = Math.max(0, state.detailSeek - 1); state.detailSeek = null; }, { once: true });
    }
    v.play();
  } catch (e) {
    toast(String(e), true);
  } finally {
    play.disabled = false;
    play.querySelector("span").textContent = "Play";
  }
}

// ---------- keys ----------

document.addEventListener("keydown", (e) => {
  const typing = e.target.closest("input, select, textarea, [contenteditable]");
  if (typing || document.querySelector("dialog[open]:not(#import-sheet)")) return;
  const sheetOpen = $("#import-sheet").open;
  if (!sheetOpen && e.metaKey && e.key.toLowerCase() === "z") {
    undoRedo(e.shiftKey ? "redo" : "undo");
    return e.preventDefault();
  }
  if (e.key === "Escape") {
    if (!$("#menu").hidden) return closeMenu();
    if (!sheetOpen && state.screen === "detail") return closeDetail();
    if (!sheetOpen && state.screen === "library" && state.selected.size) return clearSelection();
  }
  if (sheetOpen) {
    if (state.step === "review" && rTrim && rTrim.handleKey(e)) e.preventDefault();
    return;
  }
  if (state.screen === "detail") {
    if (dTrim && dTrim.handleKey(e)) return e.preventDefault();
    if (e.key === " " && !e.target.closest("button")) { playDetail(); return e.preventDefault(); }
    if (e.key === "Enter" && !e.target.closest("button")) { startRename(state.detailId); return e.preventDefault(); }
    if (libKeys(e, [state.detailId])) e.preventDefault();
    return;
  }
  if (state.screen !== "library") return;
  const ids = selectedIds();
  if ((e.metaKey && e.key === "a")) {
    selectAll();
    return e.preventDefault();
  }
  if (["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown"].includes(e.key)) {
    const list = visible().map((c) => c.id);
    if (!list.length) return;
    const i = Math.max(0, list.indexOf(state.cursor ?? state.anchor));
    const cols = settings.libView === "list" ? 1 : Math.max(1, Math.round($("#lib-content .grid")?.clientWidth / ($("#lib-content .card")?.clientWidth + 16)) || 1);
    const step = { ArrowLeft: -1, ArrowRight: 1, ArrowUp: -cols, ArrowDown: cols }[e.key];
    const next = list[Math.min(list.length - 1, Math.max(0, i + step))];
    select(next, e.shiftKey ? { shiftKey: true } : null);
    $(`[data-id="${CSS.escape(next)}"]`)?.scrollIntoView({ block: "nearest" });
    return e.preventDefault();
  }
  if (!ids.length) return;
  if (e.key === "Enter") { startRename(ids[0]); return e.preventDefault(); }
  if (e.key === " " || e.key === "t") { openDetail(ids[0]); return e.preventDefault(); }
  if (e.metaKey && e.key === "i") { openDetail(ids[0], null, "details"); return e.preventDefault(); }
  if (e.metaKey && e.key === "r") { T.opener.revealItemInDir(libClip(ids[0]).file); return e.preventDefault(); }
  if (libKeys(e, ids)) e.preventDefault();
});

async function undoRedo(which) {
  const label = which === "undo" ? History.undoLabel() : History.redoLabel();
  try {
    const step = await History[which]();
    if (step) toast(`${which === "undo" ? "Undo" : "Redo"} ${label.toLowerCase()}`);
  } catch (e) {
    toast(String(e), true);
  }
}

function libKeys(e, ids) {
  if (e.metaKey && e.key === "Backspace") { trashClips(ids); return true; }
  if (e.metaKey || e.ctrlKey || e.altKey) return false;
  const k = e.key.toLowerCase();
  if (/^[0-5]$/.test(k)) { rate(ids, +k); return true; }
  if (k === "p") { rate(ids, null, "pick"); return true; }
  if (k === "x") { rate(ids, null, "reject"); return true; }
  if (k === "u") { rate(ids, null, "none"); return true; }
  return false;
}

// ---------- volumes and cards ----------

async function refreshVolumes() {
  state.volumes = await invoke("list_volumes");
  for (const v of state.volumes.filter((x) => x.is_card)) {
    call("card_status", { mount: v.mount }).then((st) => { state.cards.set(v.mount, st); renderSidebar(); }).catch(() => {});
  }
  for (const m of [...state.cards.keys()]) if (!state.volumes.some((v) => v.mount === m)) state.cards.delete(m);
  renderSidebar();
  if (state.screen === "first-run") renderFirstRun();
}

// ---------- import sheet ----------

const clips = () => state.session?.clips || [];
const plan = (id) => state.session?.plans.find((p) => p.id === id);
const result = (id) => [...(state.session?.results || [])].reverse().find((r) => r.id === id);
const inPhotos = (id) => (state.session?.in_photos || []).includes(id);

function openImport(step) {
  const dlg = $("#import-sheet");
  if (!dlg.open) dlg.showModal();
  if (step) setStep(step);
  else setStep(state.session ? (state.session.results.length && !sessionLeft() ? "finish" : "review") : "load");
}

function setStep(step) {
  state.step = step;
  const order = ["load", "review", "export", "finish"];
  const at = order.indexOf(step);
  for (const li of $$("#imp-steps [data-step]")) {
    const i = order.indexOf(li.dataset.step);
    li.className = i < at ? "done" : i === at ? "on" : "";
    li.querySelector(".dot").innerHTML = i < at ? ICONS.check : String(i + 1);
  }
  $("#imp-review").hidden = !(step === "review" || step === "load");
  $("#imp-export").hidden = step !== "export";
  $("#imp-finish").hidden = step !== "finish";
  $("#imp-load").hidden = step !== "load";
  $("#imp-foot").hidden = step !== "finish" && step !== "export";
  $("#back-review").hidden = step !== "finish";
  $("#done-import").hidden = step !== "finish";
  $("#foot-note").textContent = step === "finish" && state.session?.card ? "The card stays mounted until you eject it." : "";
  $("[data-action=start-over]").hidden = !state.session || step === "load" || step === "export";
  renderSource();
  if (step === "review") renderReview();
  if (step === "finish") renderFinish();
}

function renderSource() {
  const s = state.session;
  const chip = $("#imp-source");
  if (!s) { chip.hidden = true; return; }
  chip.hidden = false;
  const name = s.card_volume?.info?.volume_name || base(s.source);
  chip.replaceChildren(icon(s.card ? "sd-card" : "folder-open", "c-green"), name, el("span", { class: "muted", text: ` · ${s.card ? "DVR card" : "folder"} · ${s.clips.length} clips` }));
}

async function importFrom(mount) {
  const s = state.session;
  const vol = state.volumes.find((v) => v.mount === mount);
  if (s && vol && s.card?.volume_uuid === vol.info.volume_uuid) return openImport();
  if (s && sessionLeft() && s.results.length === 0 && s.clips.length) {
    const ok = await ask("Replace the unfinished import?", `${s.clips.length} clips are loaded and not exported. Their names, dates and cuts are lost.`, { ok: "Replace", danger: true });
    if (!ok) return;
  }
  loadSource(mount);
}

async function loadSource(path) {
  if (state.busy) return;
  state.restored = false;
  state.busy = true;
  state.session = null;
  state.staging = { phase: "stage", index: 0, total: 0, done: 0, size: 0 };
  openImport("load");
  $("#clips").replaceChildren(el("li", { class: "muted", style: "padding:1rem", text: `Copying from ${tilde(path)}…` }));
  renderLoad();
  try {
    const s = await invoke("load_source", { path });
    if (!s.clips.length) {
      toast("No clips found there.", true);
      $("#import-sheet").close();
      return;
    }
    state.session = s;
    state.selectedClip = s.clips[0]?.id ?? null;
    setStep("review");
  } catch (e) {
    toast(String(e), true);
    state.session = await invoke("get_session");
    if (state.session) setStep("review");
    else $("#import-sheet").close();
  } finally {
    state.busy = false;
    state.staging = null;
    state.tasks.delete("stage");
    state.tasks.delete("analyse");
    renderSidebar();
  }
}

function renderLoad() {
  const p = state.staging;
  if (!p) return;
  const frac = p.total ? (p.index + (p.size ? p.done / p.size : 0)) / p.total : 0;
  const ring = $("#imp-ring");
  ring.style.strokeDashoffset = String(97.4 * (1 - Math.min(1, frac)));
  const left = Math.max(0, p.total - p.index);
  if (p.phase === "stage") {
    $("#imp-left").textContent = p.total ? `Copying · ${left} left` : "Copying…";
    $("#imp-file").textContent = p.size ? `${fmtBytes(p.done)} of ${fmtBytes(p.size)}` : "";
  } else {
    $("#imp-left").textContent = `Checking · ${left} left`;
    $("#imp-file").textContent = "Probing and finding dead air";
  }
}

T.event.listen("progress", ({ payload: p }) => {
  state.staging = { ...p };
  state.tasks.set(p.phase, { done: p.index, total: p.total });
  if (state.step !== "load" && $("#import-sheet").open && state.busy) setStep("load");
  renderLoad();
  renderSidebar();
});

T.event.listen("session-changed", async () => {
  setSession(await invoke("get_session"));
});

function setSession(s) {
  const hadCard = !!state.session?.card;
  const oldResults = state.session?.results?.length || 0;
  state.session = s;
  if (!s) {
    state.restored = false;
    renderSidebar();
    return;
  }
  if (hadCard && !s.card && s.warnings.some((w) => w.includes("erased"))) {
    toast("Card erased and ejected.");
    state.session = null;
    invoke("clear_session").catch(() => {});
    refreshVolumes();
    if ($("#import-sheet").open) $("#import-sheet").close();
    return;
  }
  if (state.selectedClip == null || !s.clips.some((c) => c.id === state.selectedClip)) state.selectedClip = s.clips[0]?.id ?? null;
  renderSidebar();
  if (!$("#import-sheet").open || state.busy) return;
  // Results that arrive from an agent's export open the finish step.
  if (s.results.length && s.results.length !== oldResults && state.step === "review") return setStep("finish");
  renderSource();
  if (state.step === "review") renderReview();
  if (state.step === "finish") renderFinish();
}

// ---------- review ----------

let rTrim = null;

function renderReview() {
  const s = state.session;
  if (!s) return;
  renderSessionBar();
  const f = rememberFocus();
  renderClips();
  restoreFocus(f);
  renderReviewDetail();
}

function renderSessionBar() {
  const s = state.session;
  const plans = s.plans.filter((p) => !p.skip);
  const profiles = settings.profiles.filter((p) => p.name);
  const allProf = new Set(plans.map((p) => p.meta?.profile || ""));
  const prof = $("#imp-aircraft");
  if (document.activeElement !== prof) {
    prof.replaceChildren(el("option", { value: "", text: "Automatic" }), ...profiles.map((p) => el("option", { value: p.name, text: p.name })),
      allProf.size > 1 ? el("option", { value: "*", text: "Mixed" }) : null);
    prof.value = allProf.size > 1 ? "*" : [...allProf][0] || "";
  }
  const logModels = [...new Set(plans.map((p) => p.log_model).filter(Boolean))];
  $("#imp-aircraft-hint").replaceChildren(...(logModels.length ? [icon("radio", "c-green"), `Radio log model ${logModels.join(", ")}`] : []));
  const allPlace = new Set(plans.map((p) => p.meta?.location?.name || (p.meta?.location ? "*coords" : "")));
  const place = $("#imp-place");
  if (document.activeElement !== place) {
    place.replaceChildren(el("option", { value: "", text: "None" }), ...settings.places.map((p) => el("option", { value: p.name, text: p.name })),
      allPlace.size > 1 || allPlace.has("*coords") ? el("option", { value: "*", text: "Mixed" }) : null);
    place.value = allPlace.size > 1 || allPlace.has("*coords") ? "*" : [...allPlace][0] || "";
  }
  const dates = [...new Set(plans.map((p) => p.date))];
  const date = $("#imp-date");
  if (document.activeElement !== date) date.value = dates.length === 1 ? dates[0] : "";
  const fromLog = plans.filter((p) => p.source === "log").length;
  $("#imp-date-hint").textContent = dates.length > 1 ? `${dates.length} dates` : fromLog ? `${fromLog} of ${plans.length} from the radio log` : "Import date";
  $("#log-dir").value = tilde(s.log_dir || "");
  const sel = $("#log-day");
  sel.replaceChildren(...s.log_days.map((d) => el("option", { value: d, text: d, selected: d === s.log_day })));
  sel.hidden = s.log_days.length < 2;
  const warn = [...s.warnings, ...s.date_warnings];
  $("#date-warnings").replaceChildren(...warn.map((w) => el("li", {}, icon("danger-triangle"), w)));
  const dur = s.clips.filter((c) => !plan(c.id).skip).reduce((a, c) => a + c.duration, 0);
  const dead = s.clips.filter((c) => !plan(c.id).skip).reduce((a, c) => a + (c.signal?.dead_air || []).reduce((x, d) => x + d.end - d.start, 0), 0);
  $("#imp-sum").textContent = `${s.clips.length} clips · ${fmtDur(dur - dead)} flying · ${fmtDur(dead)} dead air`;
  const todo = s.plans.filter((p) => !p.skip).length;
  const btn = $("#imp-export-btn");
  btn.replaceChildren(`Export ${todo} clip${todo === 1 ? "" : "s"}`, icon("arrow-right"));
  btn.disabled = !state.tools || state.busy;
}

// Keeps a half-typed field intact while the list re-renders under it.
function rememberFocus() {
  const a = document.activeElement;
  if (!a || !a.closest("#clips") || !a.dataset.field) return null;
  return { id: a.closest("[data-clip]").dataset.clip, field: a.dataset.field, value: a.value, start: a.selectionStart, end: a.selectionEnd };
}

function restoreFocus(f) {
  if (!f) return;
  const input = document.querySelector(`[data-clip="${f.id}"] [data-field="${f.field}"]`);
  if (!input) return;
  input.value = f.value;
  input.focus();
  try { input.setSelectionRange(f.start, f.end); } catch { /* date inputs have no selection */ }
}

async function edit(patch) {
  try {
    setSession(await invoke("edit_plan", { patch }));
  } catch (e) {
    toast(String(e), true);
  }
}

function srcChip(p) {
  if (p.source === "log") return el("span", { class: "chip src-chip log" }, icon("radio"), p.time ? `radio log ${p.time.slice(0, 5)}` : "radio log", p.segments > 1 ? ` · ${p.segments} packs` : null);
  if (p.source === "edited") return el("span", { class: "chip src-chip edited" }, icon("pen"), "edited");
  return el("span", { class: "chip src-chip import" }, icon("calendar"), state.session.log_dir ? "no log match" : "import date");
}

const agentTag = (p) => el("span", { class: "badge agent", title: p.reason || "Suggested by an agent", text: "agent" });

function renderClips() {
  const ol = $("#clips");
  ol.replaceChildren();
  for (const c of clips()) {
    const p = plan(c.id);
    const r = result(c.id);
    const outcome = r && r.outcome !== "skipped" ? r.outcome : null;
    const unusable = c.status === "empty" || !!c.stage_error;
    const sugg = p.suggested && Object.values(p.suggested).some(Boolean);
    const li = el("li", {
      class: `rrow${p.skip ? " skipped" : ""}`, "aria-selected": String(state.selectedClip === c.id), "data-clip": c.id,
      onclick: (e) => {
        if (e.target.closest("input,button,label")) return;
        state.selectedClip = c.id;
        for (const x of ol.children) x.setAttribute("aria-selected", String(x.dataset.clip === String(c.id)));
        renderReviewDetail();
      },
    });
    const thumb = c.thumb ? el("img", { class: "rthumb", src: src(c.thumb), alt: "" }) : el("div", { class: "rthumb" });
    const name = el("input", {
      type: "text", value: p.name, placeholder: settings.defaultName || "flight", "aria-label": `Short name for ${c.name}`, spellcheck: "false",
      "data-field": "name", class: p.suggested.name ? "suggested" : false, title: p.suggested.name ? `Agent suggestion${p.reason ? ": " + p.reason : ""}` : false,
      onchange: (e) => edit({ id: c.id, name: e.target.value }),
    });
    const date = el("input", { type: "date", value: p.date, "aria-label": `Date for ${c.name}`, "data-field": "date", class: p.suggested.date ? "suggested" : false, onchange: (e) => e.target.value && edit({ id: c.id, date: e.target.value }) });
    const note = el("input", { type: "text", value: p.note, placeholder: "Note", "aria-label": `Note for ${c.name}`, list: "recent-notes", "data-field": "note", class: p.suggested.note ? "suggested" : false, onchange: (e) => { remember("notes", e.target.value); edit({ id: c.id, note: e.target.value }); } });
    const skip = el("input", { type: "checkbox", checked: p.skip, disabled: unusable, "data-field": "skip", onchange: (e) => edit({ id: c.id, skip: e.target.checked }) });
    const res = el("div", { class: "result" });
    if (outcome === "verified") {
      res.classList.add("ok");
      res.append(icon("check-circle"), base(r.output));
    } else if (outcome === "failed") {
      res.classList.add("error");
      res.textContent = r.error;
    }
    const prog = state.progress.get(c.id) || 0;
    const moments = [...(p.moments || []).filter((m) => m.end > 0 && m.start < c.duration)];
    li.append(
      thumb,
      el("div", { class: "rmain" },
        el("div", { class: "rname" }, name, sugg ? agentTag(p) : null),
        el("div", { class: "rmeta" }, date, srcChip(p), el("span", { class: "mono", text: c.name }),
          c.status !== "ok" || c.stage_error ? el("span", { class: "chip c-yellow", text: c.stage_error ? "missing" : c.detail }) : null),
        note,
        sugg && p.reason ? el("div", { class: "agent-hint", text: p.reason }) : null,
        el("progress", { max: 1, value: prog, hidden: !(prog > 0 && !outcome) }), res),
      el("div", { class: "rside" },
        el("div", { class: "top" }, el("span", { text: fmtDur(c.duration) }), minibar(c.duration || 1, c.signal?.dead_air || [])),
        el("div", { class: "mchips" }, momentChips(moments)),
        p.cuts?.length ? el("span", { class: "muted small-print" }, icon("scissors", "c-sky"), ` ${p.cuts.length} cut${p.cuts.length === 1 ? "" : "s"}`) : null),
      el("label", { class: "check" }, skip, "Skip"),
    );
    ol.append(li);
  }
}

function sessionTrimModel(c, p) {
  const r = result(c.id);
  const cuts = (p.cuts || []).map((k) => {
    const done = (r?.cuts || []).find((x) => Math.abs(x.start - k.start) < 0.001 && Math.abs(x.end - k.end) < 0.001);
    return { start: k.start, end: k.end, state: done ? (done.outcome === "verified" ? "saved" : "failed") : "new", file: done?.output, error: done?.error, agent: p.suggested?.cuts };
  });
  const moments = (p.moments || []).filter((m) => m.end > 0 && m.start < c.duration);
  return {
    key: `s:${state.session.source}:${c.id}`, duration: c.duration, moments, deadAir: c.signal?.dead_air || [], keep: c.signal?.keep || [], cuts,
    hasLog: p.source === "log" || moments.length > 0, logOffset: p.log_offset_s, logInterval: p.log_interval_s,
  };
}

function renderReviewDetail() {
  const c = clips().find((x) => x.id === state.selectedClip);
  const box = $("#review-detail");
  box.hidden = !c;
  if (!c) return;
  const p = plan(c.id);
  if (state.previewId !== c.id) {
    state.previewId = c.id;
    const v = $("#r-video");
    v.pause();
    v.hidden = true;
    v.removeAttribute("src");
    const img = $("#r-img");
    if (c.thumb) img.src = src(c.thumb);
    else img.removeAttribute("src");
    img.hidden = false;
    $("[data-action=r-play]").hidden = !c.probe;
  }
  if (!rTrim) {
    rTrim = new Trim.Editor($("#r-trim"), {
      toast,
      setCuts: async (cuts) => {
        const id = state.selectedClip;
        try {
          await Trim.apply((k, removed) => call("session_cuts", { id, cuts: k, removed_cuts: removed }), cuts);
        } catch (e) {
          toast(String(e), true);
        }
      },
      setOffset: (s) => edit({ id: state.selectedClip, log_offset_s: s }),
    });
    rTrim.attachVideo($("#r-video"));
  }
  rTrim.render(c.probe ? sessionTrimModel(c, p) : null);
  $$("#r-tabs .tab").forEach((t) => { const on = t.dataset.tab === state.rTab; t.classList.toggle("on", on); t.setAttribute("aria-selected", String(on)); });
  $("#r-details").hidden = state.rTab !== "details";
  $("#r-flight").hidden = state.rTab !== "flight";
  $("#r-file").hidden = state.rTab !== "file";
  renderMeta(c, p);
  renderFlight($("#r-flight"), p.flight, (p.moments || []).filter((m) => m.end > 0 && m.start < c.duration), null);
  const pr = c.probe || {};
  const rows = [
    ["Clip", c.rel], ["Duration", fmtDur(c.duration)], ["Frames", pr.video_packets ?? "–"], ["Size", fmtBytes(c.size)],
    ["Video", pr.width ? `${pr.width}×${pr.height} @ ${pr.fps?.toFixed(2) ?? "?"} fps` : "–"], ["Audio", pr.audio_streams ? "yes" : "none"], ["Status", c.stage_error || c.detail],
  ];
  $("#r-file").replaceChildren(...rows.flatMap(([k, v]) => [el("dt", { text: k }), el("dd", { text: String(v) })]));
}

async function playReview() {
  const c = clips().find((x) => x.id === state.selectedClip);
  if (!c) return;
  const play = $("[data-action=r-play]");
  play.disabled = true;
  play.querySelector("span").textContent = "Making preview…";
  try {
    const path = await invoke("preview", { id: c.id });
    if (state.selectedClip !== c.id) return;
    const v = $("#r-video");
    v.src = src(path);
    v.hidden = false;
    v.controls = true;
    $("#r-img").hidden = true;
    play.hidden = true;
    v.play();
  } catch (e) {
    toast(String(e), true);
  } finally {
    play.disabled = false;
    play.querySelector("span").textContent = "Play";
  }
}

// ---------- clip metadata (review) ----------

function remember(kind, value) {
  value = (value || "").trim();
  if (!value) return;
  const list = [value, ...(settings.recents[kind] || []).filter((v) => v !== value)].slice(0, 12);
  save("recents", { ...settings.recents, [kind]: list });
  fillDatalists();
}

function fillDatalists() {
  const fill = (id, vals) => $(id).replaceChildren(...vals.map((v) => el("option", { value: v })));
  fill("#places-list", settings.places.map((p) => p.name));
  fill("#recent-keywords", settings.recents.keywords || []);
  fill("#recent-authors", settings.recents.authors || []);
  fill("#recent-notes", settings.recents.notes || []);
}

const findProfile = (name) => settings.profiles.find((x) => x.name.toLowerCase() === (name || "").trim().toLowerCase());
const findPlace = (name) => settings.places.find((x) => x.name.toLowerCase() === name.trim().toLowerCase());

// The same choice the core makes at export: the clip's own, the log's model, the default.
function effectiveProfile(p) {
  if (p.meta?.profile) return { prof: findProfile(p.meta.profile), why: "chosen" };
  const m = (p.log_model || "").toLowerCase();
  const byLog = m && settings.profiles.find((x) => (x.edgetx_models || []).some((n) => n.trim().toLowerCase() === m));
  if (byLog) return { prof: byLog, why: `radio log model ${p.log_model}` };
  const d = findProfile(settings.defaultProfile);
  return { prof: d, why: d ? "default" : "" };
}

const locationText = (l) => (l ? l.name || `${l.lat.toFixed(5)}, ${l.lon.toFixed(5)}` : "");

function renderMeta(c, p) {
  const m = p.meta || {};
  const { prof, why } = effectiveProfile(p);
  const sel = $("#meta-profile");
  if (document.activeElement !== sel) {
    sel.replaceChildren(
      el("option", { value: "", text: prof && why !== "chosen" ? `Automatic · ${prof.name}` : "Automatic" }),
      ...settings.profiles.filter((x) => x.name).map((x) => el("option", { value: x.name, text: x.name, selected: m.profile && x.name.toLowerCase() === m.profile.toLowerCase() })),
    );
  }
  const active = document.activeElement;
  if (active !== $("#meta-place")) $("#meta-place").value = locationText(m.location);
  if (active !== $("#meta-keywords")) $("#meta-keywords").value = (m.keywords || []).join(", ");
  if (active !== $("#meta-author")) $("#meta-author").value = m.author || "";
  $("#meta-author").placeholder = prof?.author || "";
  $("#save-place-row").hidden = !(m.location && !m.location.name);
  const loc = m.location || (prof?.place ? settings.places.find((x) => x.name === prof.place) : null);
  const words = ["FPV", ...(prof?.keywords || []), ...(m.keywords || [])];
  const parts = [
    prof ? prof.name : "No aircraft",
    [prof?.camera_make, prof?.camera_model].filter(Boolean).join(" "),
    loc ? locationText(loc) : "no location",
    `keywords ${[...new Set(words.map((w) => w.trim()).filter(Boolean))].join(", ")} + moment kinds`,
  ].filter(Boolean);
  $("#meta-effective").textContent = `Written into the file: ${parts.join(" · ")}.`;
}

// "Field" (a saved place), "40.6892, -74.0445", or empty (no location).
function placePatch(text) {
  const t = text.trim();
  if (!t) return { place: "" };
  if (findPlace(t)) return { place: findPlace(t).name };
  const m = t.match(/^\s*(-?\d+(?:\.\d+)?)\s*[, ]\s*(-?\d+(?:\.\d+)?)\s*$/);
  if (m) return { location: { lat: +m[1], lon: +m[2] } };
  return null;
}

$("#meta-profile").addEventListener("change", (e) => edit({ id: state.selectedClip, profile: e.target.value }));
$("#meta-place").addEventListener("change", (e) => {
  const patch = placePatch(e.target.value);
  if (!patch) return toast("Type a saved place, or latitude and longitude like 40.6892, -74.0445.", true);
  edit({ id: state.selectedClip, ...patch });
});
$("#meta-keywords").addEventListener("change", (e) => {
  remember("keywords", e.target.value);
  edit({ id: state.selectedClip, keywords: e.target.value.split(",").map((x) => x.trim()).filter(Boolean) });
});
$("#meta-author").addEventListener("change", (e) => {
  remember("authors", e.target.value);
  edit({ id: state.selectedClip, author: e.target.value });
});

async function savePlace() {
  const p = plan(state.selectedClip);
  const l = p?.meta?.location;
  const name = $("#new-place-name").value.trim();
  if (!l || !name) return toast("Type a name for the place first.", true);
  if (findPlace(name)) return toast(`A place named ${name} exists already.`, true);
  await save("places", [...settings.places, { name, lat: l.lat, lon: l.lon }]);
  $("#new-place-name").value = "";
  fillDatalists();
  await edit({ id: p.id, place: name });
}

async function metaToAll() {
  const p = plan(state.selectedClip);
  if (!p) return;
  const m = p.meta || {};
  const patch = { profile: m.profile || "", keywords: m.keywords || [], author: m.author || "", ...(m.location ? { location: m.location } : { place: "" }) };
  if (await editAll(clips().filter((c) => c.id !== p.id).map((c) => ({ id: c.id, ...patch })))) toast("Applied to every clip.");
}

// One core call for many clips: one save and one re-render.
async function editAll(patches) {
  if (!patches.length) return true;
  try {
    setSession(await invoke("edit_plans", { patches }));
    return true;
  } catch (e) {
    toast(String(e), true);
    return false;
  }
}

async function forAll(patchFn) {
  await editAll(clips().filter((c) => !plan(c.id).skip).map((c) => ({ id: c.id, ...patchFn(plan(c.id)) })));
}

$("#imp-aircraft").addEventListener("change", (e) => e.target.value !== "*" && forAll(() => ({ profile: e.target.value })));
$("#imp-place").addEventListener("change", (e) => e.target.value !== "*" && forAll(() => ({ place: e.target.value })));
$("#imp-date").addEventListener("change", (e) => e.target.value && forAll(() => ({ date: e.target.value })));
$("#log-day").addEventListener("change", async (e) => setSession(await invoke("plan_dates", { logDir: settings.logDir, day: e.target.value })));

async function useLogs(dir) {
  await save("logDir", dir);
  if (state.session) setSession(await invoke("plan_dates", { logDir: dir, day: null }));
  if (dir) toast(`Radio logs: ${tilde(dir)}`);
  renderAll();
}

// ---------- export ----------

// Commits a field that still has focus, so its value is in the plan before export.
async function flushEdit() {
  const a = document.activeElement;
  if (a && a.closest("#clips") && a.dataset.field && a.dataset.field !== "skip") {
    const id = Number(a.closest("[data-clip]").dataset.clip);
    const v = a.value;
    a.blur();
    if (a.dataset.field === "date") {
      if (v) await edit({ id, date: v });
    } else {
      await edit({ id, [a.dataset.field]: v });
    }
  }
}

function renderExportList() {
  const s = state.session;
  $("#exp-files").replaceChildren(...s.clips.filter((c) => !plan(c.id).skip).map((c) => {
    const p = plan(c.id);
    const r = result(c.id);
    const prog = state.progress.get(c.id);
    const stateIcon = r ? (r.outcome === "verified" ? icon("check-circle", "c-green") : icon("close-circle", "c-red")) : prog != null ? icon("refresh-moments", "c-blue") : icon("clock", "muted");
    return el("li", { class: "clip-files" }, el("b", { text: p.name || settings.defaultName || "flight" }),
      el("ul", {}, el("li", {}, stateIcon, el("span", { class: "name", text: r?.output ? base(r.output) : c.name }),
        prog != null && !r ? el("progress", { max: 1, value: prog, style: "width:120px" }) : el("span", { class: "muted", text: r?.size ? fmtBytes(r.size) : r?.error || "" }))));
  }));
}

async function runExport() {
  if (!settings.outputDir) return toast("Choose a library folder in Settings first.", true);
  await flushEdit();
  const plans = state.session.plans;
  const todo = plans.filter((p) => !p.skip).length;
  if (!todo) {
    const ok = await ask("Every clip is skipped", "Record them all as skipped?", { ok: "Record" });
    if (!ok) return;
  }
  state.busy = true;
  setStep("export");
  $("#exp-title").textContent = `Exporting ${todo} clip${todo === 1 ? "" : "s"}`;
  $("#exp-where").textContent = `To ${tilde(settings.outputDir)} · ${settings.format.toUpperCase()}`;
  $("#import-bar").value = 0;
  renderExportList();
  let done = 0;
  const offProgress = await T.event.listen("import-progress", ({ payload: p }) => {
    const frac = p.duration ? Math.min(1, p.seconds / p.duration) : 0;
    state.progress.set(p.id, frac);
    $("#import-bar").value = todo ? (done + frac) / todo : 1;
    state.tasks.set("export", { done, total: todo });
    renderExportList();
  });
  const offResult = await T.event.listen("import-result", ({ payload: r }) => {
    if (r.outcome !== "skipped") done++;
    state.progress.delete(r.id);
    state.session.results = [...state.session.results.filter((x) => x.id !== r.id), r];
    renderExportList();
  });
  try {
    const out = await invoke("import_clips", {
      options: { output_dir: settings.outputDir, format: settings.format, encoder: settings.encoder, keep_originals: settings.keepOriginals, add_time: settings.addTime },
    });
    state.session = await invoke("get_session");
    state.lastSummary = out.summary;
    state.busy = false;
    setStep("finish");
    await loadLibrary();
    makeStrips();
  } catch (e) {
    toast(String(e), true);
    state.busy = false;
    setStep("review");
  } finally {
    offProgress();
    offResult();
    state.progress.clear();
    state.tasks.delete("export");
    state.busy = false;
  }
}

// ---------- finish ----------

function formatReady() {
  const s = state.session;
  if (!s?.card) return { ok: false, why: "Clips came from a folder, not a card." };
  if (!s.results.length) return { ok: false, why: "Export first." };
  for (const c of s.clips) {
    if (c.stage_error) return { ok: false, why: `${c.name} did not copy off the card.` };
    const res = result(c.id);
    if (!res) return { ok: false, why: `${c.name} has not been exported.` };
    if (res.outcome === "failed") return { ok: false, why: `${c.name} failed to export.` };
  }
  return { ok: true };
}

async function renderFinish() {
  const s = state.session;
  if (!s) return;
  const ok = s.results.filter((r) => r.outcome === "verified");
  const cutsOk = ok.reduce((a, r) => a + (r.cuts || []).filter((k) => k.outcome === "verified").length, 0);
  const failed = s.results.filter((r) => r.outcome === "failed").length + s.results.reduce((a, r) => a + (r.cuts || []).filter((k) => k.outcome === "failed").length, 0);
  $("#fin-title").textContent = `Exported ${ok.length} clip${ok.length === 1 ? "" : "s"}${cutsOk ? ` and ${cutsOk} cut${cutsOk === 1 ? "" : "s"}` : ""}`;
  $("#fin-fail").textContent = failed ? `${failed} did not verify` : "";
  const dirs = [...new Set(ok.map((r) => r.output.split("/").slice(0, -1).join("/")))];
  $("#fin-where").textContent = dirs.length === 1 ? `Saved to ${tilde(dirs[0])}/ · each file checked against its source` : `Saved under ${tilde(s.output_dir)} · each file checked against its source`;
  $("#fin-files").replaceChildren(...s.clips.map((c) => {
    const p = plan(c.id);
    const r = result(c.id);
    if (!r || r.outcome === "skipped") return el("li", { class: "skipped", text: `${p.name || c.name} · skipped` });
    const files = [{ name: r.output ? base(r.output) : c.name, ok: r.outcome === "verified", size: r.size, error: r.error }];
    for (const k of r.cuts || []) files.push({ name: k.output ? base(k.output) : `cut ${Trim.fmtT(k.start)}–${Trim.fmtT(k.end)}`, ok: k.outcome === "verified", size: k.size, error: k.error });
    return el("li", { class: "clip-files" }, el("b", { text: p.name || settings.defaultName || "flight" }), el("ul", {}, files.map((f) => el("li", {},
      icon(f.ok ? "check-circle" : "close-circle", f.ok ? "c-green" : "c-red"), el("span", { class: "name", text: f.name }),
      f.ok ? el("span", { class: "muted", text: fmtBytes(f.size) }) : [el("span", { class: "error", text: f.error || "failed" }), el("button", { type: "button", class: "small", onclick: () => runExport() }, "Retry")]))));
  }));
  const days = [...new Set(s.plans.filter((p) => !p.skip).map((p) => p.date))];
  $("#fin-library").textContent = `${ok.length} clip${ok.length === 1 ? "" : "s"} under ${days.map((d) => fmtDay(d)).join(", ")}, in Last import.`;
  $("#fin-album").value = settings.photosAlbum || "";
  syncPhotosButton();
  const card = $("#fin-card");
  card.hidden = !s.card;
  if (s.card) {
    $("#fin-card-name").textContent = s.card.volume_name || s.card_volume?.info?.volume_name || "Card";
    const copied = s.clips.every((c) => c.staged && !c.stage_error);
    $("#fin-card-sub").textContent = `${fmtBytes(s.card_volume?.info?.total_size || 0)} DVR card${copied ? " · every clip is on this Mac" : ""}`;
  }
  $("#format-box").hidden = !s.card;
  if (!s.card) return;
  const fr = formatReady();
  const guards = [
    ["Clips came from a card", !!s.card],
    [`All ${s.clips.length} clips copied off the card`, s.clips.every((c) => !c.stage_error)],
    ["Every clip that is not skipped verified", fr.ok],
  ];
  const draw = (extra) => $("#format-guards").replaceChildren(...[...guards, ...extra].map(([t, good]) => el("li", {}, icon(good ? "check-circle" : "close-circle", good ? "c-green" : "c-red"), t)));
  draw([]);
  state.formatOk = false;
  $("#format-status").textContent = fr.ok ? "A dialog names the disk before anything is erased. Return does not confirm." : fr.why;
  if (fr.ok) {
    try {
      const plan = await invoke("format_plan", { label: $("#format-label").value || null });
      draw([[`Same card: ${plan.disk}, volume UUID matches`, true], [`Removable, ${fmtBytes(plan.size)} (64 GB at most)`, true]]);
      state.formatOk = true;
    } catch (e) {
      draw([[String(e), false]]);
    }
  }
  syncFormatButton();
}

function syncFormatButton() {
  const b = $("[data-action=format]");
  b.disabled = !($("#format-opt").checked && state.formatOk);
  b.replaceChildren(icon(b.disabled ? "lock" : "danger-triangle"), "Format card…");
}

async function addToPhotos(ids) {
  ids = ids.filter((id) => !inPhotos(id));
  if (!ids.length) return toast("Already in Photos.");
  const status = $("#photos-status");
  status.textContent = "Adding to Photos…";
  try {
    const album = $("#fin-album").value.trim();
    const r = await invoke("add_to_photos", { ids, album: album || null });
    const where = r.album ? `the ${r.album} album` : "your Photos library";
    status.textContent = r.failed.length ? `${r.added.length} added to ${where}; ${r.failed.length} failed: ${r.failed[0][1]}` : `${r.added.length} added to ${where}.`;
    toast(status.textContent, !!r.failed.length);
  } catch (e) {
    status.textContent = String(e);
    toast(String(e), true);
  }
  state.session = await invoke("get_session");
  syncPhotosButton();
}

function syncPhotosButton() {
  const verified = (state.session?.results || []).filter((r) => r.outcome === "verified");
  const left = verified.filter((r) => !inPhotos(r.id));
  const files = left.reduce((a, r) => a + 1 + (r.cuts || []).filter((k) => k.outcome === "verified").length, 0);
  const btn = $("#photos-all");
  btn.disabled = !left.length;
  btn.textContent = !verified.length ? "Add" : !left.length ? "All in Photos" : `Add ${files} file${files === 1 ? "" : "s"}`;
}

// ---------- format ----------

function confirmText(plan) {
  const media = plan.media_name ? `, ${plan.media_name}` : "";
  const n = plan.clip_count;
  return `Erase ${plan.disk} (${plan.volume_name || "untitled"}, ${fmtBytes(plan.size)}${media}) and delete ${n} clip${n === 1 ? "" : "s"}? This cannot be undone.`;
}

async function askFormat() {
  let plan;
  try {
    plan = await invoke("format_plan", { label: $("#format-label").value || null });
  } catch (e) {
    return toast(String(e), true);
  }
  state.agentFormat = null;
  $("#confirm-agent").hidden = true;
  $("#confirm-text").textContent = confirmText(plan);
  $("#confirm-format").showModal();
}

async function doFormat() {
  const dlg = $("#confirm-format");
  const btn = $("[data-action=confirm-erase]");
  if (state.agentFormat != null) {
    const id = state.agentFormat;
    state.agentFormat = null;
    dlg.close();
    await invoke("answer_format_request", { id, approve: true });
    toast("Erasing the card…");
    return;
  }
  btn.disabled = true;
  try {
    await invoke("format_card", { label: $("#format-label").value || settings.formatLabel || "DVR" });
    dlg.close();
    $("#format-opt").checked = false;
  } catch (e) {
    dlg.close();
    toast(String(e), true);
  } finally {
    btn.disabled = false;
  }
}

// An agent asked to erase the card: the person must click Erase here.
T.event.listen("agent-format-request", ({ payload }) => {
  state.agentFormat = payload.id;
  $("#confirm-agent").hidden = false;
  $("#confirm-text").textContent = confirmText(payload.plan);
  const dlg = $("#confirm-format");
  if (!dlg.open) dlg.showModal();
});

T.event.listen("agent-format-closed", ({ payload: id }) => {
  if (state.agentFormat === id) {
    state.agentFormat = null;
    $("#confirm-format").close();
  }
});

$("#confirm-format").addEventListener("close", () => {
  if (state.agentFormat != null) {
    invoke("answer_format_request", { id: state.agentFormat, approve: false });
    state.agentFormat = null;
  }
});

// Return never confirms the erase; only a click on Erase does.
$("#confirm-format").addEventListener("keydown", (e) => {
  if (e.key === "Enter") e.preventDefault();
});

// ---------- settings ----------

let draftProfiles = [];
let draftProfile = 0;

function openSettings(sec = "library") {
  draftProfiles = structuredClone(settings.profiles);
  draftProfile = 0;
  syncSettingsUI();
  renderPlacesEditor(settings.places);
  renderProfiles();
  showSection(sec);
  if (sec === "aircraft" && !draftProfiles.length) addProfile();
  if (sec === "places" && !settings.places.length) renderPlacesEditor([{ name: "", lat: "", lon: "" }]);
  $("#settings").showModal();
}

function showSection(sec) {
  for (const b of $$("#set-nav button")) b.classList.toggle("on", b.dataset.sec === sec);
  for (const p of $$("[data-pane]")) p.hidden = p.dataset.pane !== sec;
}

function syncSettingsUI() {
  $("#out-dir").value = tilde(settings.outputDir || "");
  for (const r of $$("input[name=layout]")) r.checked = r.value === settings.libraryLayout;
  $("#set-place-folders").checked = settings.placeFolders;
  $("#keep-originals").checked = settings.keepOriginals;
  $("#format").value = settings.format;
  $("#add-time").checked = settings.addTime;
  $("#set-default-name").value = settings.defaultName;
  $("#set-album").value = settings.photosAlbum;
  $("#format-label").value = settings.formatLabel || "DVR";
  $("#set-encoder").value = settings.encoder;
  $("#set-seg-gap").value = settings.tunables.segment_gap_s;
  $("#set-session-gap").value = settings.tunables.session_gap_min;
  $("#set-tolerance").value = settings.tunables.tolerance_s;
  renderLayoutExample();
}

// The same paths `library::day_dir` makes.
function renderLayoutExample() {
  const layout = $$("input[name=layout]").find((r) => r.checked)?.value || settings.libraryLayout;
  const placeOn = $("#set-place-folders").checked;
  const keep = $("#keep-originals").checked;
  const place = settings.places[0]?.name || "Home field";
  const day = new Date().toISOString().slice(0, 10);
  const yr = day.slice(0, 4);
  const dayDir = placeOn ? `${day} ${place}` : day;
  $("#place-example").textContent = `${day} ${place}`;
  const root = `${base(settings.outputDir) || "quadcam"}/`;
  const files = [`${day}_backyard_loops.mp4`, `${day}_backyard_loops_cut1.mp4`, ...(keep ? [`originals/${day}_backyard_loops.avi`] : []), "…"];
  const tree = (indent, list) => list.map((f, i) => `${indent}${i === list.length - 1 ? "└─" : "├─"} ${f}`).join("\n");
  let text;
  if (layout === "flat") text = `${root}\n${tree("", files)}`;
  else if (layout === "day") text = `${root}\n└─ ${dayDir}/\n${tree("   ", files)}`;
  else text = `${root}\n└─ ${yr}/\n   └─ ${dayDir}/\n${tree("      ", files)}`;
  $("#layout-example").textContent = text;
}

for (const id of ["#set-place-folders", "#keep-originals"]) $(id).addEventListener("change", renderLayoutExample);
for (const r of $$("input[name=layout]")) r.addEventListener("change", renderLayoutExample);
$("#set-nav").addEventListener("click", (e) => {
  const b = e.target.closest("button[data-sec]");
  if (b) showSection(b.dataset.sec);
});

function renderPlacesEditor(places) {
  $("#places-editor").replaceChildren(...places.map((p, i) => el("div", { class: "row", "data-place": i },
    el("input", { type: "text", value: p.name, placeholder: "Name", "data-k": "name", "aria-label": "Place name" }),
    el("input", { type: "number", step: "any", value: p.lat, placeholder: "Latitude", "data-k": "lat", "aria-label": "Latitude" }),
    el("input", { type: "number", step: "any", value: p.lon, placeholder: "Longitude", "data-k": "lon", "aria-label": "Longitude" }),
    el("button", { type: "button", class: "icon small", "aria-label": "Delete this place", title: "Delete", onclick: () => renderPlacesEditor(readPlaces().filter((_, j) => j !== i)) }, icon("trash-bin-trash")))));
}

function readPlaces() {
  return $$("[data-place]").map((r) => ({
    name: r.querySelector("[data-k=name]").value.trim(),
    lat: parseFloat(r.querySelector("[data-k=lat]").value),
    lon: parseFloat(r.querySelector("[data-k=lon]").value),
  }));
}

const VIDEO_SYSTEMS = ["Analog", "DJI O4", "Walksnail", "HDZero"];

function renderProfiles() {
  const counts = new Map((state.lib?.groups?.aircraft || []).map(([a, n]) => [a, n]));
  $("#profile-list").replaceChildren(
    ...draftProfiles.map((p, i) => el("li", {}, el("button", { type: "button", class: i === draftProfile ? "on" : "", onclick: () => { readProfileForm(); draftProfile = i; renderProfiles(); } },
      el("b", {}, icon("quad", "c-pink"), p.name || "New aircraft"),
      el("span", { class: "sub", text: [p.video_system, p.name && p.name === settings.defaultProfile ? "default" : null, counts.get(p.name) ? `${counts.get(p.name)} clips` : null].filter(Boolean).join(" · ") })))),
    el("li", {}, el("button", { type: "button", onclick: addProfile }, el("b", {}, icon("add", "c-blue"), "Add aircraft"))));
  const form = $("#profile-form");
  const p = draftProfiles[draftProfile];
  if (!p) return form.replaceChildren();
  const fld = (label, k, wide, attrs = {}) => el("label", { class: `field${wide ? " wide" : ""}` }, el("span", { text: label }), el("input", { type: "text", value: Array.isArray(p[k]) ? p[k].join(", ") : p[k] || "", "data-pk": k, spellcheck: "false", ...attrs }));
  const seg = el("div", { class: "segbtns", role: "group", "aria-label": "Video system" }, VIDEO_SYSTEMS.map((v) => el("button", {
    type: "button", "aria-pressed": String((p.video_system || "Analog") === v), onclick: () => { readProfileForm(); p.video_system = v; renderProfiles(); },
  }, v)));
  const placeSel = el("select", { "data-pk": "place" }, el("option", { value: "", text: "None" }), ...settings.places.map((x) => el("option", { value: x.name, text: x.name, selected: x.name === p.place })));
  const def = el("input", { type: "checkbox", id: "profile-default", checked: !!p.name && p.name === (state.draftDefault ?? settings.defaultProfile) });
  form.replaceChildren(
    fld("Name", "name"), fld("Aircraft", "aircraft"),
    el("div", { class: "field wide" }, el("span", { text: "Video system" }), seg),
    fld("Camera make", "camera_make"), fld("Camera model", "camera_model"),
    fld("Radio model names", "edgetx_models", true, { placeholder: "AIR65, …" }),
    el("label", { class: "field" }, el("span", { text: "Default place" }), placeSel),
    fld("Author", "author"),
    fld("Keywords", "keywords", true, { placeholder: "Comma-separated" }),
    el("div", { class: "row wide" }, el("label", { class: "check" }, def, el("span", {}, el("b", { text: "Default aircraft" }), el("span", { class: "muted", text: "For clips without a matching radio log." }))),
      el("button", { type: "button", class: "ghost small", style: "margin-left:auto", onclick: () => { draftProfiles.splice(draftProfile, 1); draftProfile = 0; renderProfiles(); } }, icon("trash-bin-trash", "c-red"), "Delete")));
  def.addEventListener("change", () => { readProfileForm(); state.draftDefault = def.checked ? p.name : ""; });
}

function readProfileForm() {
  const p = draftProfiles[draftProfile];
  if (!p) return;
  for (const input of $$("#profile-form [data-pk]")) {
    const k = input.dataset.pk;
    const v = input.value.trim();
    p[k] = ["keywords", "edgetx_models"].includes(k) ? v.split(",").map((x) => x.trim()).filter(Boolean) : v;
  }
  p.place = p.place || null;
  p.video_system = p.video_system || "Analog";
}

function addProfile() {
  readProfileForm();
  draftProfiles.push({ name: "", aircraft: "", camera_make: "", camera_model: "", video_system: "Analog", keywords: [], author: "", place: null, edgetx_models: [] });
  draftProfile = draftProfiles.length - 1;
  renderProfiles();
}

$("#settings").addEventListener("close", async () => {
  readProfileForm();
  const num = (id, d) => (Number.isFinite(+$(id).value) && $(id).value !== "" ? +$(id).value : d);
  await save("defaultName", $("#set-default-name").value.trim() || "flight");
  await save("encoder", $("#set-encoder").value);
  await save("format", $("#format").value);
  await save("addTime", $("#add-time").checked);
  await save("photosAlbum", $("#set-album").value.trim());
  await save("libraryLayout", $$("input[name=layout]").find((r) => r.checked)?.value || "year_day");
  await save("placeFolders", $("#set-place-folders").checked);
  await save("keepOriginals", $("#keep-originals").checked);
  const places = readPlaces().filter((p) => p.name && Number.isFinite(p.lat) && Number.isFinite(p.lon) && Math.abs(p.lat) <= 90 && Math.abs(p.lon) <= 180);
  if (places.length < readPlaces().filter((p) => p.name || Number.isFinite(p.lat)).length) toast("Places without a name or a valid latitude and longitude were not saved.", true);
  await save("places", places);
  const profiles = draftProfiles.filter((p) => p.name);
  await save("profiles", profiles);
  if (state.draftDefault !== undefined) await save("defaultProfile", state.draftDefault || "");
  else if (!profiles.some((p) => p.name === settings.defaultProfile)) await save("defaultProfile", profiles[0]?.name || "");
  state.draftDefault = undefined;
  fillDatalists();
  await save("tunables", { ...settings.tunables, segment_gap_s: num("#set-seg-gap", 5), session_gap_min: num("#set-session-gap", 20), tolerance_s: num("#set-tolerance", 30) });
  if (state.session) setSession(await invoke("plan_dates", { logDir: settings.logDir, day: state.session.log_day }));
  await loadLibrary();
});

// ---------- wiring ----------

async function pickFolder(title, current) {
  return T.dialog.open({ directory: true, multiple: false, title, defaultPath: current || undefined });
}

const actions = {
  "open-folder": async () => {
    const p = await pickFolder("Folder with DVR clips");
    if (p) importFrom(p);
  },
  import: () => {
    if (state.session && (sessionLeft() || !state.session.results.length)) return openImport();
    const card = state.volumes.find((v) => v.is_card);
    if (card) return importFrom(card.mount);
    actions["open-folder"]();
  },
  "close-import": () => $("#import-sheet").close(),
  settings: () => openSettings("library"),
  "settings-aircraft": () => openSettings("aircraft"),
  "settings-places": () => openSettings("places"),
  "settings-library": () => openSettings("library"),
  "add-place": () => renderPlacesEditor([...readPlaces(), { name: "", lat: "", lon: "" }]),
  "save-place": savePlace,
  "meta-all": metaToAll,
  "pick-logs": async () => {
    const p = await pickFolder("Radio log folder (LOGS or the radio's root)", settings.logDir);
    if (p) useLogs(p);
  },
  "clear-logs": () => useLogs(null),
  "pick-output": async () => {
    const p = await pickFolder("Library folder", settings.outputDir);
    if (!p) return;
    await save("outputDir", p);
    $("#out-dir").value = tilde(p);
    invoke("library_scope").catch(() => {});
    await loadLibrary();
  },
  "reveal-library": () => settings.outputDir && T.opener.openPath(settings.outputDir),
  rebuild: async () => {
    try {
      const r = await call("library_rebuild");
      toast(`${r.clips} clip${r.clips === 1 ? "" : "s"} and ${r.cuts} cut${r.cuts === 1 ? "" : "s"} in the library.${r.problems.length ? ` ${r.problems.length} file${r.problems.length === 1 ? "" : "s"} left out.` : ""}`, r.problems.length > 0);
      await loadLibrary();
      makeStrips();
    } catch (e) {
      toast(String(e), true);
    }
  },
  "view-grid": () => { save("libView", "grid"); renderLibrary(); },
  "view-list": () => { save("libView", "list"); renderLibrary(); },
  "run-export": runExport,
  "r-play": playReview,
  "d-play": playDetail,
  "photos-all": () => addToPhotos((state.session?.results || []).filter((r) => r.outcome === "verified").map((r) => r.id)),
  format: askFormat,
  "confirm-erase": doFormat,
  eject: async () => {
    try {
      await invoke("eject", { path: null });
      toast("Card ejected.");
      refreshVolumes();
    } catch (e) {
      toast(String(e), true);
    }
  },
  "back-review": () => setStep("review"),
  "done-import": async () => {
    $("#import-sheet").close();
    state.filter = { group: "last_import" };
    state.screen = "library";
    await loadLibrary();
  },
  "start-over": async () => {
    if (state.busy || !state.session) return;
    const left = sessionLeft();
    const ok = await ask("Start over?", left ? `${left} clip${left > 1 ? "s are" : " is"} not exported yet; their names, dates and cuts are lost. Exported files stay.` : "This clears the loaded clips. Exported files stay.", { ok: "Start over", danger: true });
    if (!ok) return;
    try {
      await invoke("clear_session");
      state.session = null;
      state.restored = false;
      state.selectedClip = null;
      $("#import-sheet").close();
      refreshVolumes();
    } catch (e) {
      toast(String(e), true);
    }
  },
};

document.addEventListener("click", (e) => {
  const b = e.target.closest("[data-action]");
  if (b && !b.disabled && actions[b.dataset.action]) actions[b.dataset.action](e);
});

for (const tabs of ["#r-tabs", "#d-tabs"]) {
  $(tabs).addEventListener("click", (e) => {
    const t = e.target.closest("[data-tab]");
    if (!t) return;
    if (tabs === "#r-tabs") { state.rTab = t.dataset.tab; renderReviewDetail(); } else { state.dTab = t.dataset.tab; renderDetail(); }
  });
}

$("#sort").addEventListener("change", (e) => { if (e.target.value !== librarySort().key) setSort(e.target.value); });
$("#search").addEventListener("input", (e) => { state.query = e.target.value; if (state.screen === "library") renderLibrary(); });
$("#thumb-size").addEventListener("input", (e) => { settings.thumbSize = +e.target.value; renderLibrary(); });
$("#thumb-size").addEventListener("change", (e) => save("thumbSize", +e.target.value));
$("#format-opt").addEventListener("change", syncFormatButton);
$("#format-label").addEventListener("input", (e) => {
  e.target.value = e.target.value.toUpperCase().replace(/[^A-Z0-9_-]/g, "").slice(0, 11);
});
$("#format-label").addEventListener("change", (e) => e.target.value && save("formatLabel", e.target.value));
$("#fin-album").addEventListener("change", (e) => save("photosAlbum", e.target.value.trim()));
// Closing the sheet keeps the session; Escape closes it unless a step is running.
$("#import-sheet").addEventListener("cancel", (e) => { if (state.busy) e.preventDefault(); });

T.event.listen("volumes-changed", refreshVolumes);
T.event.listen("library-changed", () => loadLibrary());
T.event.listen("library-task", ({ payload: t }) => {
  state.tasks.set(t.task, { done: t.done, total: t.total });
  if (t.done >= t.total) setTimeout(() => { state.tasks.delete(t.task); renderSidebar(); }, 800);
  renderSidebar();
});
window.addEventListener("focus", () => loadLibrary());

async function init() {
  hydrateIcons();
  await loadSettings();
  // Until the user picks a folder, the library is ~/Movies/quadcam (created on first import).
  if (!settings.outputDir) settings.outputDir = await invoke("default_output_dir");
  state.home = (settings.outputDir.match(/^\/Users\/[^/]+/) || [""])[0];
  $("#thumb-size").value = settings.thumbSize || 3;
  fillDatalists();
  await pushDefaults();
  invoke("library_scope").catch(() => {});
  const env = await invoke("env_check");
  state.tools = env.tools;
  if (!env.tools) {
    banner(`<b>ffmpeg not found.</b> Import is blocked. Install it with <code>${env.install_hint}</code>, then restart quadcam.`, "error");
    for (const b of $$("[data-action=import]")) b.disabled = true;
  }
  $("#tools-status").textContent = [env.tools ? `ffmpeg: ${env.tools.ffmpeg}` : env.error, env.socket ? `Agent socket: ${env.socket}` : ""].filter(Boolean).join("\n");
  const s = await invoke("get_session");
  if (s) {
    state.session = s;
    state.restored = true;
    state.selectedClip = s.clips[0]?.id ?? null;
  }
  await loadLibrary();
  await refreshVolumes();
  makeStrips();
}

init();
