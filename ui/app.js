// quadcam frontend. Plain JS, no build step. The Rust core owns the session (clips, plans,
// results); this page renders it, sends the person's edits back, and re-renders when the
// core changes, including changes an agent makes over the control socket.
"use strict";

const T = window.__TAURI__;
const invoke = T.core.invoke;
const $ = (s, root = document) => root.querySelector(s);

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
  tunables: { segment_gap_s: 5, session_gap_min: 20, tolerance_s: 30, max_log_age_days: 60 },
  // Saved places [{name, lat, lon}] and aircraft profiles; see Profile in metadata.rs.
  places: [],
  profiles: [],
  defaultProfile: "",
  // Recently used values, offered as suggestions.
  recents: { keywords: [], authors: [], notes: [] },
};

// "Format card" is deliberately not a setting: it is never remembered as on.
const settings = structuredClone(DEFAULTS);
let store = null;

const state = {
  tools: null,
  volumes: [],
  busy: false,
  view: "empty",
  session: null,
  selected: null,
  progress: new Map(), // clip id -> 0..1 while converting
  lastSummary: null,
  agentFormat: null, // id of an agent's pending format request
  previewId: null, // clip shown in the preview pane
  trim: { id: null, in: null, out: null }, // in/out being edited (not saved until "Add cut")
};

// ---------- helpers ----------

function fmtBytes(n) {
  if (!n) return "0 B";
  const u = ["B", "KB", "MB", "GB", "TB"];
  const i = Math.min(u.length - 1, Math.floor(Math.log10(n) / 3));
  return `${(n / 10 ** (3 * i)).toFixed(i > 1 ? 1 : 0)} ${u[i]}`;
}

function fmtDur(s) {
  if (!s || s <= 0) return "–";
  s = Math.round(s);
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
}

// Seconds as m:ss.s.
function fmtT(s) {
  s = Math.max(0, s || 0);
  const m = Math.floor(s / 60);
  return `${m}:${(s - m * 60).toFixed(1).padStart(4, "0")}`;
}

function el(tag, attrs = {}, ...kids) {
  const e = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs)) {
    if (v === false || v == null) continue;
    if (k === "class") e.className = v;
    else if (k === "text") e.textContent = v;
    else if (k.startsWith("on")) e.addEventListener(k.slice(2), v);
    else e.setAttribute(k, v === true ? "" : v);
  }
  for (const kid of kids.flat()) if (kid != null) e.append(kid);
  return e;
}

function icon(name) {
  const i = el("i", { "data-icon": name });
  i.innerHTML = ICONS[name] || "";
  return i;
}

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

function show(view) {
  state.view = view;
  $("#empty-state").hidden = view !== "empty";
  $("#stage-state").hidden = view !== "stage";
  $("#workspace").hidden = view !== "workspace";
  $("#summary").hidden = view !== "summary";
}

function setBusy(b) {
  state.busy = b;
  for (const btn of document.querySelectorAll("[data-action=import],[data-action=open-folder],.vol button")) btn.disabled = b;
  if (!state.tools) $("[data-action=import]").disabled = true;
}

const clips = () => state.session?.clips || [];
const plan = (id) => state.session?.plans.find((p) => p.id === id);
const result = (id) => [...(state.session?.results || [])].reverse().find((r) => r.id === id);
const inPhotos = (id) => (state.session?.in_photos || []).includes(id);

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
    },
  });
}

async function save(k, v) {
  settings[k] = v;
  if (store) await store.set(k, v);
  await pushDefaults();
}

function syncSettingsUI() {
  $("#out-dir").value = settings.outputDir || "";
  $("#format").value = settings.format;
  $("#keep-originals").checked = settings.keepOriginals;
  $("#add-time").checked = settings.addTime;
  $("#log-dir").value = settings.logDir || "";
  $("#set-default-name").value = settings.defaultName;
  $("#set-album").value = settings.photosAlbum;
  $("#format-label").value = settings.formatLabel || "DVR";
  $("#set-encoder").value = settings.encoder;
  $("#set-seg-gap").value = settings.tunables.segment_gap_s;
  $("#set-session-gap").value = settings.tunables.session_gap_min;
  $("#set-tolerance").value = settings.tunables.tolerance_s;
}

// ---------- volumes ----------

async function refreshVolumes() {
  state.volumes = await invoke("list_volumes");
  const ul = $("#volumes");
  ul.replaceChildren();
  for (const v of state.volumes) {
    const name = v.info.volume_name || v.mount.split("/").pop();
    if (v.is_card) {
      ul.append(
        el("li", { class: "vol card" }, icon("sd-card"), name,
          el("span", { class: "sub", text: `${v.info.parent_whole_disk} · ${fmtBytes(v.info.total_size)}` }),
          el("button", { type: "button", class: "primary", onclick: () => loadSource(v.mount), disabled: state.busy }, "Import")),
      );
    } else if (v.is_radio) {
      ul.append(
        el("li", { class: "vol radio" }, icon("radio"), name,
          el("button", { type: "button", class: "ghost small", onclick: () => useLogs(v.mount) }, "Use logs")),
      );
    }
  }
  // A card that shows up while the app is idle starts staging at once.
  const card = state.volumes.find((v) => v.is_card);
  if (card && state.view === "empty" && !state.busy && state.tools) loadSource(card.mount);
}

// ---------- session ----------

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

function setSession(s, { quiet = false } = {}) {
  const hadCard = !!state.session?.card;
  const oldResults = state.session?.results?.length || 0;
  state.session = s;
  if (!s) {
    show("empty");
    return;
  }
  if (hadCard && !s.card && s.warnings.some((w) => w.includes("erased"))) {
    toast("Card erased and ejected. It is ready for the goggles.");
    state.session = null;
    show("empty");
    refreshVolumes();
    return;
  }
  if (state.selected == null || !s.clips.some((c) => c.id === state.selected)) state.selected = s.clips[0]?.id ?? null;
  if (state.view === "empty" || state.view === "stage") {
    if (s.analysed) show("workspace");
  }
  const f = rememberFocus();
  renderDates();
  renderClips();
  restoreFocus(f);
  if (!quiet || state.previewId !== state.selected) renderPreview();
  else { renderEditor(); renderMeta(); }
  // Results that arrive from an agent's import open the summary too.
  if (s.results.length && s.results.length !== oldResults && !state.busy) {
    state.lastSummary = null;
    renderSummary();
    show("summary");
  } else if (state.view === "summary") {
    renderSummary();
  }
}

async function loadSource(path) {
  if (state.busy) return;
  setBusy(true);
  show("stage");
  $("#stage-title").textContent = "Copying clips off the card…";
  $("#stage-bar").value = 0;
  $("#stage-detail").textContent = path;
  try {
    const s = await invoke("load_source", { path });
    if (!s.clips.length) {
      toast("No clips found there.", true);
      show("empty");
      return;
    }
    show("workspace");
    setSession(s);
  } catch (e) {
    toast(String(e), true);
    show(state.session ? "workspace" : "empty");
  } finally {
    setBusy(false);
  }
}

T.event.listen("progress", ({ payload: p }) => {
  if (state.view !== "stage") show("stage");
  if (p.phase === "stage") {
    $("#stage-title").textContent = `Copying clip ${p.index + 1} of ${p.total}…`;
    const frac = p.size ? p.done / p.size : 1;
    $("#stage-bar").value = p.total ? (p.index + frac) / p.total : 0;
    $("#stage-detail").textContent = `${fmtBytes(p.done)} of ${fmtBytes(p.size)}`;
  } else {
    $("#stage-title").textContent = `Checking clip ${p.index + 1} of ${p.total}…`;
    $("#stage-bar").value = p.total ? p.index / p.total : 0;
    $("#stage-detail").textContent = "Probing, recovering half-written files, making thumbnails";
  }
});

T.event.listen("session-changed", async () => {
  setSession(await invoke("get_session"), { quiet: true });
});

// ---------- dates ----------

async function useLogs(dir) {
  await save("logDir", dir);
  $("#log-dir").value = dir || "";
  if (state.session) setSession(await invoke("plan_dates", { logDir: dir, day: null }));
}

function renderDates() {
  const s = state.session;
  const sel = $("#log-day");
  sel.replaceChildren(...s.log_days.map((d) => el("option", { value: d, text: d, selected: d === s.log_day })));
  sel.disabled = !s.log_days.length;
  const warn = [...s.warnings, ...s.date_warnings];
  $("#date-warnings").replaceChildren(...warn.map((w) => el("li", {}, icon("danger-triangle"), w)));
  const anyLog = s.plans.some((p) => p.source === "log");
  $("#add-time").disabled = !anyLog;
  $("#add-time-wrap").style.opacity = anyLog ? 1 : 0.5;
}

async function edit(patch) {
  try {
    setSession(await invoke("edit_plan", { patch }), { quiet: true });
  } catch (e) {
    toast(String(e), true);
  }
}

// ---------- clip list ----------

function statusBadge(c) {
  if (c.stage_error) return el("span", { class: "badge missing", text: "missing" });
  if (c.status === "ok") return el("span", { class: "badge ok", text: "OK" });
  if (c.status === "incomplete") return el("span", { class: "badge incomplete", text: c.detail });
  return el("span", { class: "badge empty", text: c.detail === "zero bytes" ? "empty" : c.detail || "empty" });
}

function sourceLine(p) {
  if (p.source === "log") {
    const t = p.time ? ` ${p.time.slice(0, 5)}` : "";
    const packs = p.segments > 1 ? `, ${p.segments} packs` : "";
    return el("span", { class: "src" }, `radio log${t}${packs}`, el("span", { class: `badge ${p.badge}`, text: p.badge }));
  }
  if (p.source === "edited") return el("span", { class: "src" }, p.suggested.date ? agentTag(p) : null, "edited");
  const unmatched = state.session.log_dir ? el("span", { class: "badge unmatched", text: "unmatched" }) : null;
  return el("span", { class: "src" }, "import date", unmatched);
}

function agentTag(p) {
  return el("span", { class: "badge agent", title: p.reason || "Suggested by an agent", text: "agent" });
}

function renderClips() {
  const ol = $("#clips");
  ol.replaceChildren();
  for (const c of clips()) {
    const p = plan(c.id);
    const r = result(c.id);
    const outcome = r && r.outcome !== "skipped" ? r.outcome : null;
    const unusable = c.status === "empty" || !!c.stage_error;
    const li = el("li", {
      class: `clip${p.skip ? " skipped" : ""}${p.suggested && Object.values(p.suggested).some(Boolean) ? " has-suggestion" : ""}`,
      "aria-selected": String(state.selected === c.id),
      "data-outcome": outcome || false,
      "data-clip": c.id,
      onclick: (e) => {
        if (e.target.closest("input,button,label")) return;
        state.selected = c.id;
        for (const x of ol.children) x.setAttribute("aria-selected", String(x.dataset.clip === String(c.id)));
        renderPreview();
      },
    });
    const thumb = c.thumb ? el("img", { class: "thumb", src: T.core.convertFileSrc(c.thumb), alt: `First frame of ${c.name}` }) : el("div", { class: "thumb" });
    const date = el("input", {
      type: "date", value: p.date, "aria-label": `Date for ${c.name}`, "data-field": "date",
      class: p.suggested.date ? "suggested" : false,
      onchange: (e) => e.target.value && edit({ id: c.id, date: e.target.value }),
    });
    const name = el("input", {
      type: "text", value: p.name, placeholder: settings.defaultName || "flight", "aria-label": `Short name for ${c.name}`, spellcheck: "false",
      "data-field": "name", class: p.suggested.name ? "suggested" : false, title: p.suggested.name ? `Agent suggestion${p.reason ? ": " + p.reason : ""}` : false,
      onchange: (e) => edit({ id: c.id, name: e.target.value }),
    });
    const note = el("input", {
      type: "text", value: p.note, placeholder: "Note (metadata only)", "aria-label": `Note for ${c.name}`, list: "recent-notes",
      "data-field": "note", class: p.suggested.note ? "suggested" : false,
      onchange: (e) => { remember("notes", e.target.value); edit({ id: c.id, note: e.target.value }); },
    });
    const skip = el("input", {
      type: "checkbox", checked: p.skip, disabled: unusable, "data-field": "skip",
      onchange: (e) => edit({ id: c.id, skip: e.target.checked }),
    });
    const res = el("div", { class: "result" });
    if (outcome === "verified") {
      res.classList.add("ok");
      const cutsOk = (r.cuts || []).filter((x) => x.outcome === "verified").length;
      const cutsBad = (r.cuts || []).filter((x) => x.outcome === "failed").length;
      res.textContent = `Verified → ${r.output.split("/").pop()}${cutsOk ? ` + ${cutsOk} cut${cutsOk > 1 ? "s" : ""}` : ""}${cutsBad ? ` (${cutsBad} cut failed)` : ""}`;
      res.append(
        inPhotos(c.id)
          ? el("span", { class: "src", text: " · in Photos" })
          : el("button", { type: "button", class: "ghost small", onclick: () => addToPhotos([c.id]) }, icon("gallery-add"), "Add to Photos"),
      );
    } else if (outcome === "failed") {
      res.classList.add("error");
      res.textContent = r.error;
    }
    const prog = state.progress.get(c.id) || 0;
    const bar = el("progress", { max: 1, value: prog, hidden: !(prog > 0 && !outcome) });
    const hint = p.suggested && Object.values(p.suggested).some(Boolean)
      ? el("div", { class: "agent-hint" }, agentTag(p), p.reason ? ` ${p.reason}` : " Suggested by an agent. Edit any field to make it yours.")
      : null;
    li.append(
      thumb,
      el("div", {},
        el("div", { class: "clip-head" },
          el("strong", { text: c.name }),
          el("span", { class: "facts", text: `${fmtDur(c.duration)} · ${fmtBytes(c.size)}${c.rel !== c.name ? " · " + c.rel : ""}${countLine(c, p)}` }),
          statusBadge(c)),
        el("div", { class: "clip-fields" },
          el("div", { class: "date-cell" }, date, sourceLine(p)),
          name, note,
          el("label", { class: "check" }, skip, "Skip")),
        hint, bar, res),
    );
    ol.append(li);
  }
}

function renderPreview() {
  state.previewId = state.selected;
  const c = clips().find((x) => x.id === state.selected);
  const img = $("#preview-img");
  const video = $("#preview-video");
  video.pause();
  video.hidden = true;
  video.removeAttribute("src");
  const play = $("[data-action=play]");
  if (!c) {
    img.removeAttribute("src");
    play.hidden = true;
    $("#preview-meta").replaceChildren();
    return;
  }
  if (c.thumb) img.src = T.core.convertFileSrc(c.thumb);
  else img.removeAttribute("src");
  img.hidden = false;
  play.hidden = !c.probe;
  const p = c.probe || {};
  const rows = [
    ["Clip", c.rel],
    ["Duration", fmtDur(c.duration)],
    ["Frames", p.video_packets ?? "–"],
    ["Size", fmtBytes(c.size)],
    ["Video", p.width ? `${p.width}×${p.height} @ ${p.fps?.toFixed(2) ?? "?"} fps` : "–"],
    ["Audio", p.audio_streams ? "yes" : "none"],
    ["Status", c.stage_error || c.detail],
  ];
  $("#preview-meta").replaceChildren(...rows.flatMap(([k, v]) => [el("dt", { text: k }), el("dd", { text: String(v) })]));
  renderEditor();
  renderMeta();
}

// ---------- moments and cuts ----------

function countLine(c, p) {
  const n = clipMoments(c, p).length;
  const k = p.cuts?.length || 0;
  return `${n ? ` · ${n} moment${n > 1 ? "s" : ""}` : ""}${k ? ` · ${k} cut${k > 1 ? "s" : ""}` : ""}`;
}

// Log moments inside the clip plus dead air, by start time.
function clipMoments(c, p) {
  const log = (p?.moments || []).filter((m) => m.end > 0 && m.start < c.duration);
  return [...log, ...(c.signal?.dead_air || [])].sort((a, b) => a.start - b.start);
}

const KIND = { roll: "roll", flip: "flip", punch: "punch-out", dive: "dive", crash: "crash?", dead_air: "dead air" };

function playhead() {
  const v = $("#preview-video");
  return v.src && !v.hidden ? v.currentTime : null;
}

function seek(t) {
  const v = $("#preview-video");
  if (v.src && !v.hidden) v.currentTime = Math.max(0, t);
}

function trimFor(id) {
  if (state.trim.id !== id) state.trim = { id, in: null, out: null };
  return state.trim;
}

function renderEditor() {
  const c = clips().find((x) => x.id === state.selected);
  const box = $("#editor");
  const p = c && plan(c.id);
  box.hidden = !c || !c.probe || !(c.duration > 0);
  if (box.hidden) return;
  const dur = c.duration;
  const pct = (t) => `${(Math.min(Math.max(t, 0), dur) / dur) * 100}%`;
  const width = (a, b) => `${((Math.min(b, dur) - Math.max(a, 0)) / dur) * 100}%`;
  const tr = trimFor(c.id);
  const moments = clipMoments(c, p);
  const tl = $("#timeline");
  const parts = [];
  for (const d of c.signal?.dead_air || []) parts.push(el("div", { class: "dead", style: `left:${pct(d.start)};width:${width(d.start, d.end)}`, title: `Dead air ${fmtT(d.start)}–${fmtT(d.end)}: ${d.detail}` }));
  for (const k of c.signal?.keep || []) parts.push(el("div", { class: "keep", style: `left:${pct(k.start)};width:${width(k.start, k.end)}`, title: `Suggested keep ${fmtT(k.start)}–${fmtT(k.end)}` }));
  for (const k of p.cuts || []) parts.push(el("div", { class: "cut", style: `left:${pct(k.start)};width:${width(k.start, k.end)}` }));
  if (tr.in != null && tr.out != null && tr.out > tr.in) parts.push(el("div", { class: "sel", style: `left:${pct(tr.in)};width:${width(tr.in, tr.out)}` }));
  for (const m of moments.filter((m) => m.kind !== "dead_air")) {
    parts.push(el("button", {
      type: "button", class: `mark${m.score < 0.5 ? " low" : ""}`, style: `left:${pct(m.start)};width:${width(m.start, m.end)}`,
      title: `${KIND[m.kind]} at ${fmtT(m.start)} (${Math.round(m.score * 100)}%): ${m.detail}`, "aria-label": `${KIND[m.kind]} at ${fmtT(m.start)}`,
      onclick: (e) => { e.stopPropagation(); pickMoment(c, m); },
    }));
  }
  const head = el("div", { class: "head", hidden: playhead() == null });
  parts.push(head);
  tl.replaceChildren(...parts);
  tl.onclick = (e) => {
    const r = tl.getBoundingClientRect();
    seek(((e.clientX - r.left) / r.width) * dur);
  };
  $("#moment-list").replaceChildren(...moments.map((m) => el("li", {},
    el("button", { type: "button", class: m.kind === "dead_air" ? "dead" : "", title: m.detail, onclick: () => pickMoment(c, m) },
      `${KIND[m.kind]} ${fmtT(m.start)}`))));
  $("#cut-in").value = tr.in ?? "";
  $("#cut-out").value = tr.out ?? "";
  $("#cut-in").max = $("#cut-out").max = dur;
  $("#use-keep").hidden = !(c.signal?.keep?.length);
  const hasLog = p.source === "log" || (p.moments || []).length > 0;
  $("#offset-wrap").hidden = $("#arm-here").hidden = !hasLog;
  $("#log-offset").value = p.log_offset_s ?? 0;
  const r = result(c.id);
  $("#cut-list").replaceChildren(...(p.cuts || []).map((k, i) => {
    const done = (r?.cuts || []).find((x) => Math.abs(x.start - k.start) < 0.001 && Math.abs(x.end - k.end) < 0.001);
    return el("li", {},
      el("span", { text: `Cut ${i + 1}  ${fmtT(k.start)}–${fmtT(k.end)}  (${(k.end - k.start).toFixed(1)} s)` }),
      p.suggested?.cuts ? agentTag(p) : null,
      done ? el("span", { class: done.outcome === "verified" ? "ok" : "error", text: done.outcome === "verified" ? done.output.split("/").pop() : done.error }) : null,
      el("button", { type: "button", class: "icon small", title: "Remove this cut", onclick: () => setCuts(c.id, p.cuts.filter((_, j) => j !== i)) }, icon("close-square")));
  }));
  const notes = [];
  if (p.log_interval_s > 0.3) notes.push(`Radio log rows are ${p.log_interval_s.toFixed(1)} s apart, so moment times are rough. Set the EdgeTX log interval to 0.1 s for better detection.`);
  if (hasLog) notes.push("Log moments assume the clip starts at arm. Play to where you armed and click Arm is here to line them up.");
  $("#moment-note").textContent = notes.join(" ");
}

// ---------- clip metadata ----------

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

// The same choice the core makes at export: the clip's own, the log's model, the default.
function effectiveProfile(p) {
  if (p.meta?.profile) return { prof: findProfile(p.meta.profile), why: "chosen" };
  const m = (p.log_model || "").toLowerCase();
  const byLog = m && settings.profiles.find((x) => (x.edgetx_models || []).some((n) => n.trim().toLowerCase() === m));
  if (byLog) return { prof: byLog, why: `radio log model ${p.log_model}` };
  const d = findProfile(settings.defaultProfile);
  return { prof: d, why: d ? "default" : "" };
}

function locationText(l) {
  if (!l) return "";
  return l.name || `${l.lat.toFixed(5)}, ${l.lon.toFixed(5)}`;
}

function renderMeta() {
  const c = clips().find((x) => x.id === state.selected);
  const box = $("#clip-meta");
  box.hidden = !c;
  if (!c) return;
  const p = plan(c.id);
  const m = p.meta || {};
  const { prof, why } = effectiveProfile(p);
  const sel = $("#meta-profile");
  sel.replaceChildren(
    el("option", { value: "", text: prof && why !== "chosen" ? `Automatic: ${prof.name} (${why})` : "Automatic (radio log model, then default)" }),
    ...settings.profiles.map((x) => el("option", { value: x.name, text: x.name, selected: m.profile && x.name.toLowerCase() === m.profile.toLowerCase() })),
  );
  const active = document.activeElement;
  if (active !== $("#meta-place")) $("#meta-place").value = locationText(m.location);
  if (active !== $("#meta-keywords")) $("#meta-keywords").value = (m.keywords || []).join(", ");
  if (active !== $("#meta-author")) $("#meta-author").value = m.author || "";
  $("#meta-author").placeholder = prof?.author || "";
  $("#save-place-row").hidden = !(m.location && !m.location.name);
  const loc = m.location || (prof?.place ? settings.places.find((x) => x.name === prof.place) : null);
  const words = ["FPV", ...(prof?.keywords || []), ...(m.keywords || [])];
  const parts = [
    prof ? `Profile ${prof.name}` : "No profile",
    [prof?.camera_make, prof?.camera_model].filter(Boolean).join(" "),
    loc ? `at ${locationText(loc)}` : "no location",
    `keywords ${[...new Set(words.map((w) => w.trim()).filter(Boolean))].join(", ")} + moment kinds`,
  ].filter(Boolean);
  if (p.flight) parts.push(`log: ${flightLine(p.flight)}`);
  $("#meta-effective").textContent = `Export writes: ${parts.join(" · ")}.`;
}

function flightLine(f) {
  const s = Math.round(f.armed_s);
  const bits = [`armed ${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")} over ${f.packs} pack${f.packs === 1 ? "" : "s"}`];
  if (f.min_rx_bat_v != null) bits.push(`min RxBt ${f.min_rx_bat_v.toFixed(2)} V`);
  if (f.min_lq != null) bits.push(`min LQ ${Math.round(f.min_lq)}%`);
  if (f.max_throttle != null) bits.push(`max throttle ${Math.round(f.max_throttle * 100)}%`);
  return bits.join(", ");
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
const findPlace = (name) => settings.places.find((x) => x.name.toLowerCase() === name.trim().toLowerCase());

$("#meta-profile").addEventListener("change", (e) => edit({ id: state.selected, profile: e.target.value }));
$("#meta-place").addEventListener("change", (e) => {
  const patch = placePatch(e.target.value);
  if (!patch) return toast("Type a saved place, or latitude and longitude like 40.6892, -74.0445.", true);
  edit({ id: state.selected, ...patch });
});
$("#meta-keywords").addEventListener("change", (e) => {
  remember("keywords", e.target.value);
  edit({ id: state.selected, keywords: e.target.value.split(",").map((x) => x.trim()).filter(Boolean) });
});
$("#meta-author").addEventListener("change", (e) => {
  remember("authors", e.target.value);
  edit({ id: state.selected, author: e.target.value });
});

async function savePlace() {
  const c = clips().find((x) => x.id === state.selected);
  const l = c && plan(c.id).meta?.location;
  const name = $("#new-place-name").value.trim();
  if (!l || !name) return toast("Type a name for the place first.", true);
  if (findPlace(name)) return toast(`A place named ${name} exists already.`, true);
  await save("places", [...settings.places, { name, lat: l.lat, lon: l.lon }]);
  $("#new-place-name").value = "";
  fillDatalists();
  await edit({ id: c.id, place: name });
}

async function metaToAll() {
  const c = clips().find((x) => x.id === state.selected);
  if (!c) return;
  const m = plan(c.id).meta || {};
  const patch = {
    profile: m.profile || "",
    keywords: m.keywords || [],
    author: m.author || "",
    ...(m.location ? { location: m.location } : { place: "" }),
  };
  for (const other of clips()) if (other.id !== c.id) await edit({ id: other.id, ...patch });
  toast("Metadata applied to every clip.");
}

// ---------- settings: places and profiles ----------

function renderPlacesEditor(places) {
  $("#places-editor").replaceChildren(...places.map((p, i) => el("div", { class: "row", "data-place": i },
    el("input", { type: "text", value: p.name, placeholder: "Name", "data-k": "name", "aria-label": "Place name" }),
    el("input", { type: "number", step: "any", value: p.lat, placeholder: "Latitude", "data-k": "lat", "aria-label": "Latitude" }),
    el("input", { type: "number", step: "any", value: p.lon, placeholder: "Longitude", "data-k": "lon", "aria-label": "Longitude" }),
    el("button", { type: "button", class: "icon small", title: "Delete this place", onclick: () => renderPlacesEditor(readPlaces().filter((_, j) => j !== i)) }, icon("close-square")))));
}

function readPlaces() {
  return [...document.querySelectorAll("[data-place]")].map((r) => ({
    name: r.querySelector("[data-k=name]").value.trim(),
    lat: parseFloat(r.querySelector("[data-k=lat]").value),
    lon: parseFloat(r.querySelector("[data-k=lon]").value),
  }));
}

const PROFILE_FIELDS = [
  ["name", "Name"], ["aircraft", "Aircraft"], ["camera_make", "Camera make (goggles or DVR)"], ["camera_model", "Camera model"],
  ["video_system", "Video system"], ["keywords", "Keywords (comma-separated)"], ["author", "Author"], ["place", "Default place"],
  ["edgetx_models", "EdgeTX model names (comma-separated)"],
];
const LIST_FIELDS = ["keywords", "edgetx_models"];

function renderProfilesEditor(profiles) {
  $("#profiles-editor").replaceChildren(...profiles.map((p, i) => el("details", { "data-profile": i, open: !p.name },
    el("summary", { text: p.name || "New profile" }),
    ...PROFILE_FIELDS.map(([k, label]) => {
      const v = LIST_FIELDS.includes(k) ? (p[k] || []).join(", ") : p[k] || "";
      const input = k === "video_system"
        ? el("select", { "data-k": k }, ...["", "Analog", "DJI", "Walksnail", "HDZero"].map((x) => el("option", { value: x, text: x || "–", selected: x === v })))
        : el("input", { type: "text", value: v, "data-k": k, list: k === "place" ? "places-list" : false, spellcheck: "false" });
      return el("label", { class: "field" }, el("span", { text: label }), input);
    }),
    el("button", { type: "button", class: "ghost small", onclick: () => renderProfilesEditor(readProfiles().filter((_, j) => j !== i)) }, icon("trash-bin-trash"), "Delete profile"))));
  const sel = $("#set-default-profile");
  sel.replaceChildren(el("option", { value: "", text: "None" }), ...profiles.filter((p) => p.name).map((p) => el("option", { value: p.name, text: p.name, selected: p.name === settings.defaultProfile })));
}

function readProfiles() {
  return [...document.querySelectorAll("[data-profile]")].map((d) => {
    const p = {};
    for (const [k] of PROFILE_FIELDS) {
      const v = d.querySelector(`[data-k=${k}]`).value.trim();
      p[k] = LIST_FIELDS.includes(k) ? v.split(",").map((x) => x.trim()).filter(Boolean) : v;
    }
    p.place = p.place || null;
    return p;
  });
}

function pickMoment(c, m) {
  const pad = m.kind === "dead_air" ? 0 : 1;
  const tr = trimFor(c.id);
  tr.in = +Math.max(0, m.start - pad).toFixed(1);
  tr.out = +Math.min(c.duration, m.end + pad).toFixed(1);
  seek(tr.in);
  renderEditor();
}

async function setCuts(id, cuts) {
  await edit({ id, cuts: cuts.map((k) => ({ start: +k.start, end: +k.end })) });
  renderEditor();
}

function addCut() {
  const c = clips().find((x) => x.id === state.selected);
  if (!c) return;
  const tr = trimFor(c.id);
  const a = parseFloat($("#cut-in").value);
  const b = parseFloat($("#cut-out").value);
  if (!Number.isFinite(a) || !Number.isFinite(b) || b - a < 0.5) {
    toast("Set an in and an out point at least 0.5 s apart.", true);
    return;
  }
  tr.in = tr.out = null;
  setCuts(c.id, [...(plan(c.id).cuts || []), { start: a, end: b }]);
}

function setPoint(which) {
  const c = clips().find((x) => x.id === state.selected);
  const t = playhead();
  if (!c) return;
  if (t == null) {
    toast("Play the clip first, or type the seconds.", true);
    return;
  }
  trimFor(c.id)[which] = +t.toFixed(1);
  renderEditor();
}

$("#preview-video").addEventListener("timeupdate", () => {
  const c = clips().find((x) => x.id === state.selected);
  const head = $("#timeline .head");
  if (!c || !head) return;
  head.hidden = false;
  head.style.left = `${(Math.min($("#preview-video").currentTime, c.duration) / c.duration) * 100}%`;
});
for (const [id, k] of [["#cut-in", "in"], ["#cut-out", "out"]]) {
  $(id).addEventListener("change", (e) => {
    const v = parseFloat(e.target.value);
    trimFor(state.selected)[k] = Number.isFinite(v) ? v : null;
    renderEditor();
  });
}
$("#log-offset").addEventListener("change", (e) => {
  const v = parseFloat(e.target.value);
  if (Number.isFinite(v)) edit({ id: state.selected, log_offset_s: v });
});

async function playPreview() {
  const c = clips().find((x) => x.id === state.selected);
  if (!c) return;
  const play = $("[data-action=play]");
  play.disabled = true;
  play.querySelector("span").textContent = "Making preview…";
  try {
    const path = await invoke("preview", { id: c.id });
    if (state.selected !== c.id) return;
    const video = $("#preview-video");
    video.src = T.core.convertFileSrc(path);
    video.hidden = false;
    $("#preview-img").hidden = true;
    play.hidden = true;
    video.play();
  } catch (e) {
    toast(String(e), true);
  } finally {
    play.disabled = false;
    play.querySelector("span").textContent = "Play";
  }
}

// ---------- import ----------

// Commits a field that still has focus, so its value is in the plan before import.
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

async function runImport() {
  if (!settings.outputDir) {
    toast("Choose an output folder first.", true);
    return;
  }
  await flushEdit();
  const plans = state.session.plans;
  const todo = plans.filter((p) => !p.skip).length;
  if (!todo && !confirm("Every clip is skipped. Record them all as skipped?")) return;
  setBusy(true);
  $("#import-bar").hidden = false;
  $("#import-bar").value = 0;
  let done = 0;
  const offProgress = await T.event.listen("import-progress", ({ payload: p }) => {
    const frac = p.duration ? Math.min(1, p.seconds / p.duration) : 0;
    state.progress.set(p.id, frac);
    const bar = document.querySelector(`[data-clip="${p.id}"] progress`);
    if (bar) { bar.hidden = false; bar.value = frac; }
    $("#import-bar").value = todo ? (done + frac) / todo : 1;
    $("#export-note").textContent = `Converting ${clips().find((c) => c.id === p.id)?.name}…`;
  });
  const offResult = await T.event.listen("import-result", ({ payload: r }) => {
    if (r.outcome !== "skipped") done++;
    state.progress.delete(r.id);
  });
  try {
    const out = await invoke("import_clips", {
      options: {
        output_dir: settings.outputDir,
        format: settings.format,
        encoder: settings.encoder,
        keep_originals: settings.keepOriginals,
        add_time: settings.addTime,
      },
    });
    state.session = await invoke("get_session");
    state.lastSummary = out.summary;
    renderClips();
    renderSummary();
    show("summary");
  } catch (e) {
    toast(String(e), true);
  } finally {
    offProgress();
    offResult();
    state.progress.clear();
    $("#import-bar").hidden = true;
    $("#export-note").textContent = "";
    setBusy(false);
  }
}

function summaryNow() {
  const s = state.session;
  const count = (o) => s.results.filter((r) => r.outcome === o).length;
  return {
    results: s.results,
    imported: count("verified"),
    skipped: count("skipped"),
    failed: count("failed"),
    total_bytes: s.results.reduce((a, r) => a + (r.size || 0), 0),
    output_dir: s.output_dir,
  };
}

function formatReady() {
  const s = state.session;
  if (!s?.card) return { ok: false, why: "Clips came from a folder, not a card." };
  if (!s.results.length) return { ok: false, why: "Import first." };
  const r = state.lastSummary?.format_ready;
  if (r && "Err" in r) return { ok: false, why: r.Err };
  for (const c of s.clips) {
    if (c.stage_error) return { ok: false, why: `${c.name} did not copy off the card.` };
    const res = result(c.id);
    if (!res) return { ok: false, why: `${c.name} has not been imported.` };
    if (res.outcome === "failed") return { ok: false, why: `${c.name} failed to import.` };
  }
  return { ok: true };
}

function renderSummary() {
  if (!state.session) return;
  const s = summaryNow();
  $("#counts").replaceChildren(
    el("li", { class: "good" }, el("b", { text: s.imported }), el("span", { text: "imported" })),
    el("li", {}, el("b", { text: s.skipped }), el("span", { text: "skipped" })),
    el("li", { class: s.failed ? "bad" : "" }, el("b", { text: s.failed }), el("span", { text: "failed" })),
    el("li", {}, el("b", { text: fmtBytes(s.total_bytes) }), el("span", { text: "written" })),
  );
  $("#out-link").textContent = s.output_dir ? `Open ${s.output_dir}` : "";
  $("#failures").replaceChildren(
    ...s.results.filter((r) => r.outcome === "failed").map((r) => el("li", { class: "error" }, icon("close-circle"), `${clips().find((c) => c.id === r.id)?.name}: ${r.error}`)),
  );
  syncPhotosButton();
  $("#format-box").hidden = !state.session.card;
  const fr = formatReady();
  $("#format-status").textContent = fr.ok
    ? "Every non-skipped clip verified. The card can be formatted. The app checks every guard again before it erases."
    : `Format locked: ${fr.why}`;
  syncFormatButton();
}

function syncFormatButton() {
  $("[data-action=format]").disabled = !($("#format-opt").checked && formatReady().ok);
}

// ---------- Photos ----------

async function addToPhotos(ids) {
  ids = ids.filter((id) => !inPhotos(id));
  if (!ids.length) {
    toast("Those clips are already in Photos.");
    return;
  }
  const status = $("#photos-status");
  status.textContent = "Adding to Photos…";
  try {
    const r = await invoke("add_to_photos", { ids, album: settings.photosAlbum || null });
    const where = r.album ? `the "${r.album}" album` : "your Photos library";
    if (r.failed.length) {
      const msg = `${r.added.length} added to ${where}; ${r.failed.length} failed: ${r.failed[0][1]}`;
      status.textContent = msg;
      toast(msg, true);
    } else {
      status.textContent = `${r.added.length} added to ${where}.`;
      toast(status.textContent);
    }
  } catch (e) {
    status.textContent = String(e);
    toast(String(e), true);
  }
  state.session = await invoke("get_session");
  renderClips();
  syncPhotosButton();
}

function syncPhotosButton() {
  const verified = (state.session?.results || []).filter((r) => r.outcome === "verified");
  const left = verified.filter((r) => !inPhotos(r.id)).length;
  const btn = $("[data-action=photos-all]");
  btn.disabled = !left;
  btn.lastChild.textContent = !verified.length ? "Add to Photos" : !left ? "All in Photos" : left > 1 ? `Add all ${left} to Photos` : "Add it to Photos";
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
    toast(String(e), true);
    return;
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

// ---------- wiring ----------

async function pickFolder(title, current) {
  return T.dialog.open({ directory: true, multiple: false, title, defaultPath: current || undefined });
}

const actions = {
  "open-folder": async () => {
    const p = await pickFolder("Folder with DVR clips");
    if (p) loadSource(p);
  },
  "refresh-volumes": refreshVolumes,
  settings: () => {
    syncSettingsUI();
    renderPlacesEditor(settings.places);
    renderProfilesEditor(settings.profiles);
    $("#settings").showModal();
  },
  "add-place": () => renderPlacesEditor([...readPlaces(), { name: "", lat: "", lon: "" }]),
  "add-profile": () => renderProfilesEditor([...readProfiles(), { name: "" }]),
  "save-place": savePlace,
  "meta-all": metaToAll,
  "pick-logs": async () => {
    const p = await pickFolder("Radio log folder (LOGS or the radio's root)", settings.logDir);
    if (p) useLogs(p);
  },
  "clear-logs": () => useLogs(null),
  "apply-date": async () => {
    await flushEdit();
    const plans = state.session?.plans || [];
    const first = plans.find((p) => !p.skip);
    if (!first) return;
    for (const p of plans) {
      if (p === first || p.skip) continue;
      await edit({ id: p.id, date: first.date });
    }
  },
  "pick-output": async () => {
    const p = await pickFolder("Output folder", settings.outputDir);
    if (p) { await save("outputDir", p); $("#out-dir").value = p; }
  },
  import: runImport,
  play: playPreview,
  "set-in": () => setPoint("in"),
  "set-out": () => setPoint("out"),
  "add-cut": addCut,
  "use-keep": () => {
    const c = clips().find((x) => x.id === state.selected);
    if (c?.signal?.keep?.length) setCuts(c.id, c.signal.keep);
  },
  "arm-here": () => {
    const t = playhead();
    if (t == null) return toast("Play the clip to where you armed first.", true);
    edit({ id: state.selected, log_offset_s: +t.toFixed(1) });
  },
  "photos-all": () => addToPhotos((state.session?.results || []).filter((r) => r.outcome === "verified").map((r) => r.id)),
  "reveal-output": () => state.session?.output_dir && T.opener.openPath(state.session.output_dir),
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
  back: () => { show("workspace"); renderClips(); },
};

document.addEventListener("click", (e) => {
  const b = e.target.closest("[data-action]");
  if (b && !b.disabled && actions[b.dataset.action]) actions[b.dataset.action](e);
});

// Return never confirms the erase; only a click on Erase does.
$("#confirm-format").addEventListener("keydown", (e) => {
  if (e.key === "Enter") e.preventDefault();
});

$("#format").addEventListener("change", (e) => save("format", e.target.value));
$("#keep-originals").addEventListener("change", (e) => save("keepOriginals", e.target.checked));
$("#add-time").addEventListener("change", (e) => save("addTime", e.target.checked));
$("#format-opt").addEventListener("change", syncFormatButton);
$("#format-label").addEventListener("input", (e) => {
  e.target.value = e.target.value.toUpperCase().replace(/[^A-Z0-9_-]/g, "").slice(0, 11);
});
$("#format-label").addEventListener("change", (e) => e.target.value && save("formatLabel", e.target.value));
$("#log-day").addEventListener("change", async (e) => {
  setSession(await invoke("plan_dates", { logDir: settings.logDir, day: e.target.value }));
});
$("#settings").addEventListener("close", async () => {
  const num = (id, d) => (Number.isFinite(+$(id).value) && $(id).value !== "" ? +$(id).value : d);
  await save("defaultName", $("#set-default-name").value.trim() || "flight");
  await save("encoder", $("#set-encoder").value);
  await save("photosAlbum", $("#set-album").value.trim());
  const places = readPlaces().filter((p) => p.name && Number.isFinite(p.lat) && Number.isFinite(p.lon) && Math.abs(p.lat) <= 90 && Math.abs(p.lon) <= 180);
  if (places.length < readPlaces().length) toast("Places without a name or a valid latitude and longitude were not saved.", true);
  await save("places", places);
  await save("profiles", readProfiles().filter((p) => p.name));
  await save("defaultProfile", $("#set-default-profile").value);
  fillDatalists();
  await save("tunables", {
    ...settings.tunables,
    segment_gap_s: num("#set-seg-gap", 5),
    session_gap_min: num("#set-session-gap", 20),
    tolerance_s: num("#set-tolerance", 30),
  });
  if (state.session) setSession(await invoke("plan_dates", { logDir: settings.logDir, day: state.session.log_day }));
});

T.event.listen("volumes-changed", refreshVolumes);

async function init() {
  hydrateIcons();
  await loadSettings();
  // Until the user picks a folder, output goes to ~/Movies/quadcam (created on first import).
  if (!settings.outputDir) settings.outputDir = await invoke("default_output_dir");
  syncSettingsUI();
  fillDatalists();
  await pushDefaults();
  const env = await invoke("env_check");
  state.tools = env.tools;
  if (!env.tools) {
    banner(`<b>ffmpeg not found.</b> Import is blocked. Install it with <code>${env.install_hint}</code>, then restart quadcam.`, "error");
    $("[data-action=import]").disabled = true;
  }
  $("#tools-status").textContent = [env.tools ? `ffmpeg: ${env.tools.ffmpeg}` : env.error, env.socket ? `agent socket: ${env.socket}` : ""].filter(Boolean).join("\n");
  show("empty");
  const s = await invoke("get_session");
  if (s) setSession(s);
  await refreshVolumes();
}

init();
