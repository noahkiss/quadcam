// The one trim editor. The library's clip detail and the import sheet's review panel both
// use it, so a cut behaves the same in both places: the timeline (dead air, keep ranges,
// moments, cuts, playhead), in and out points with drag handles, the cut list, the log
// offset, the keys, and the question when a cut that was already exported is removed.
// The host passes callbacks that reach the core; this file never calls the core itself.
"use strict";

const Trim = (() => {
  const KIND = { roll: "Roll", flip: "Flip", punch: "Punch-out", dive: "Dive", crash: "Crash?", dead_air: "Dead air" };
  const KIND_ICON = { roll: "roll", flip: "flip", punch: "bolt", dive: "dive", crash: "danger-triangle", dead_air: "close-circle" };
  const FRAME = 1 / 30;

  function fmtT(s, tenths = true) {
    s = Math.max(0, s || 0);
    const m = Math.floor(s / 60);
    const rest = s - m * 60;
    return tenths ? `${m}:${rest.toFixed(1).padStart(4, "0")}` : `${m}:${String(Math.floor(rest)).padStart(2, "0")}`;
  }

  // "1:02.5", "62.5" or "" -> seconds or null.
  function parseT(text) {
    const t = String(text || "").trim();
    if (!t) return null;
    const m = t.match(/^(\d+):(\d+(?:\.\d+)?)$/);
    if (m) return +m[1] * 60 + +m[2];
    const n = Number(t);
    return Number.isFinite(n) ? n : null;
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

  function icon(name) {
    const i = el("i", { "data-icon": name });
    i.innerHTML = (typeof ICONS !== "undefined" && ICONS[name]) || "";
    return i;
  }

  function kbd(k) {
    return el("kbd", { text: k });
  }

  // Asks what happens to exported files whose cut is removed. Resolves "keep", "trash" or null.
  function askRemoved(files) {
    const dlg = document.getElementById("removed-cuts");
    const list = dlg.querySelector("[data-files]");
    list.replaceChildren(...files.map((f) => el("li", { class: "mono", text: f.split("/").pop() })));
    dlg.querySelector("[data-count]").textContent = files.length === 1 ? "This cut is already a file." : `These ${files.length} cuts are already files.`;
    return new Promise((resolve) => {
      const done = (v) => {
        dlg.removeEventListener("close", onClose);
        resolve(v);
      };
      const onClose = () => done(["keep", "trash"].includes(dlg.returnValue) ? dlg.returnValue : null);
      dlg.addEventListener("close", onClose);
      dlg.returnValue = "";
      dlg.showModal();
    });
  }

  // Sends a cut list through `call(cuts, decision)`. When the core answers that exported
  // files would lose their cut, asks the person, then sends it again with the answer.
  async function apply(call, cuts) {
    let r = await call(cuts, null);
    if (r && r.status === "confirm") {
      const decision = await askRemoved(r.files);
      if (!decision) return null;
      r = await call(cuts, decision);
    }
    return r;
  }

  class Editor {
    // opts: { setCuts(cuts) -> Promise, setOffset(s) -> Promise, save() -> Promise (optional),
    //         toast(msg, isError), momentsEl (optional element for the moment list) }
    constructor(root, opts) {
      this.root = root;
      this.opts = opts;
      this.model = null;
      this.sel = { in: null, out: null };
      this.key = null; // which clip the selection belongs to
      this.video = null;
      this.onTime = () => this.syncHead();
      root.classList.add("trim");
      this.tl = el("div", { class: "tl", role: "group", "aria-label": "Clip timeline. Click to seek." });
      this.legend = el("div", { class: "tl-legend" });
      this.inInput = el("input", { type: "text", class: "mono t-in", "aria-label": "In point", placeholder: "0:00.0", spellcheck: "false" });
      this.outInput = el("input", { type: "text", class: "mono t-out", "aria-label": "Out point", placeholder: "0:00.0", spellcheck: "false" });
      this.keepBtn = el("button", { type: "button", class: "ghost small", onclick: () => this.useKeep() });
      this.saveBtn = el("button", { type: "button", class: "primary small", onclick: () => this.opts.save && this.opts.save() });
      this.list = el("ol", { class: "cut-list", "aria-label": "Cuts" });
      this.offset = el("input", { type: "number", step: "0.1", class: "mono", "aria-label": "Log offset in seconds" });
      this.offsetRow = el("div", { class: "offset-row" },
        el("span", { text: "Log starts at" }), this.offset, el("span", { text: "s" }),
        el("button", { type: "button", class: "ghost small", title: "Sets the log start to the playhead", onclick: () => this.armHere() }, "Arm is here"));
      this.note = el("p", { class: "trim-note muted" });
      const bar = el("div", { class: "trim-bar" },
        el("h3", { text: "Cuts" }),
        el("label", { class: "inline" }, "In", this.inInput),
        el("button", { type: "button", class: "ghost small", onclick: () => this.setPoint("in") }, "Set in ", kbd("I")),
        el("label", { class: "inline" }, "Out", this.outInput),
        el("button", { type: "button", class: "ghost small", onclick: () => this.setPoint("out") }, "Set out ", kbd("O")),
        el("button", { type: "button", class: "small", onclick: () => this.addCut() }, icon("add"), "Add cut"),
        this.keepBtn, this.saveBtn);
      this.keys = el("p", { class: "keys", "aria-label": "Keys" },
        el("span", {}, kbd("Space"), "play"),
        el("span", {}, kbd("J"), kbd("K"), kbd("L"), "back, stop, forward"),
        el("span", {}, kbd("←"), kbd("→"), "one frame"),
        el("span", {}, kbd("I"), kbd("O"), "set in, out"),
        el("span", {}, kbd("C"), "add cut"));
      root.replaceChildren(this.tl, this.legend, bar, this.list, this.offsetRow, this.note, this.keys);
      for (const [input, k] of [[this.inInput, "in"], [this.outInput, "out"]]) {
        input.addEventListener("change", () => {
          this.sel[k] = parseT(input.value);
          this.render(this.model);
        });
      }
      this.offset.addEventListener("change", () => {
        const v = parseFloat(this.offset.value);
        if (Number.isFinite(v)) this.opts.setOffset(v);
      });
      this.tl.addEventListener("click", (e) => {
        if (!this.model || e.target.closest("button")) return;
        const r = this.tl.getBoundingClientRect();
        this.seek(((e.clientX - r.left) / r.width) * this.model.duration);
      });
    }

    attachVideo(video) {
      if (this.video) this.video.removeEventListener("timeupdate", this.onTime);
      this.video = video;
      if (video) video.addEventListener("timeupdate", this.onTime);
      this.syncHead();
    }

    playhead() {
      const v = this.video;
      return v && v.src && !v.hidden ? v.currentTime : null;
    }

    seek(t) {
      const v = this.video;
      if (v && v.src && !v.hidden) v.currentTime = Math.max(0, Math.min(t, this.model?.duration || t));
    }

    syncHead() {
      const head = this.tl.querySelector(".head");
      const t = this.playhead();
      if (!head || !this.model) return;
      head.hidden = t == null;
      if (t != null) {
        head.style.left = `${(Math.min(t, this.model.duration) / this.model.duration) * 100}%`;
        head.dataset.t = fmtT(t, false);
      }
    }

    // model: { key, duration, moments, deadAir, keep, cuts: [{start, end, state, file, error}],
    //          logOffset, hasLog, logInterval, saveLabel }
    render(model) {
      this.model = model;
      if (!model || !(model.duration > 0)) {
        this.root.hidden = true;
        return;
      }
      this.root.hidden = false;
      if (this.key !== model.key) {
        this.key = model.key;
        this.sel = { in: null, out: null };
      }
      const dur = model.duration;
      const pct = (t) => `${(Math.min(Math.max(t, 0), dur) / dur) * 100}%`;
      const width = (a, b) => `${((Math.min(b, dur) - Math.max(a, 0)) / dur) * 100}%`;
      const parts = [];
      // Ruler: about eight ticks at round steps.
      const steps = [1, 2, 5, 10, 15, 30, 60, 120, 300, 600];
      const step = steps.find((s) => dur / s <= 8) || 600;
      const ruler = el("div", { class: "ruler", "aria-hidden": "true" });
      for (let t = 0; t <= dur + 0.001; t += step) ruler.append(el("span", { style: `left:${pct(t)}`, text: fmtT(t, false) }));
      parts.push(ruler);
      const track = el("div", { class: "track" });
      for (const d of model.deadAir || []) {
        track.append(el("div", { class: "dead", style: `left:${pct(d.start)};width:${width(d.start, d.end)}`, title: `Dead air ${fmtT(d.start)}–${fmtT(d.end)}${d.detail ? ": " + d.detail : ""}` },
          el("span", { text: `${d.detail || "Dead air"} · ${fmtT(d.end - d.start, false)}` })));
      }
      model.cuts.forEach((k, i) => track.append(el("div", { class: `cutzone${k.state === "new" ? " new" : ""}`, style: `left:${pct(k.start)};width:${width(k.start, k.end)}`, title: `Cut ${i + 1}` })));
      if (this.sel.in != null && this.sel.out != null && this.sel.out > this.sel.in) {
        const sel = el("div", { class: "sel", style: `left:${pct(this.sel.in)};width:${width(this.sel.in, this.sel.out)}` });
        track.append(sel);
        for (const which of ["in", "out"]) {
          const h = el("button", { type: "button", class: `handle ${which}`, "aria-label": which === "in" ? "Drag the in point" : "Drag the out point", style: `left:${pct(this.sel[which])}` });
          h.addEventListener("pointerdown", (e) => this.drag(e, which));
          h.addEventListener("keydown", (e) => {
            if (e.key !== "ArrowLeft" && e.key !== "ArrowRight") return;
            e.preventDefault();
            e.stopPropagation();
            this.sel[which] = Math.max(0, Math.min(dur, this.sel[which] + (e.key === "ArrowLeft" ? -0.1 : 0.1)));
            this.render(this.model);
            this.tl.querySelector(`.handle.${which}`)?.focus();
          });
          track.append(h);
        }
      }
      parts.push(track);
      const marks = el("div", { class: "marks" });
      for (const m of model.moments || []) {
        const low = m.score < 0.5 ? " low" : "";
        marks.append(el("button", {
          type: "button", class: `mark k-${m.kind}${low}`, style: `left:${pct(m.start)}`,
          title: `${KIND[m.kind]} at ${fmtT(m.start)} · score ${Math.round(m.score * 100)} %`, "aria-label": `${KIND[m.kind]} at ${fmtT(m.start)}`,
          onclick: (e) => { e.stopPropagation(); this.frame(m); },
        }, icon(KIND_ICON[m.kind])));
      }
      parts.push(marks);
      const lanes = el("div", { class: "lanes", "aria-hidden": "true" });
      for (const k of model.keep || []) lanes.append(el("span", { class: "keep", style: `left:${pct(k.start)};width:${width(k.start, k.end)}` }));
      model.cuts.forEach((k, i) => lanes.append(el("span", { class: `cut${k.state === "new" ? " new" : ""}`, style: `left:${pct(k.start)};width:${width(k.start, k.end)}`, text: String(i + 1) })));
      parts.push(lanes);
      parts.push(el("span", { class: "head", hidden: true }));
      this.tl.replaceChildren(...parts);
      this.syncHead();

      const deadSecs = (model.deadAir || []).reduce((a, d) => a + (Math.min(d.end, dur) - Math.max(d.start, 0)), 0);
      const keepSecs = (model.keep || []).reduce((a, k) => a + (k.end - k.start), 0);
      this.legend.replaceChildren(...[
        el("span", {}, el("i", { class: "sw fly" }), "Flying"),
        deadSecs ? el("span", {}, el("i", { class: "sw dead" }), `Dead air ${fmtT(deadSecs, false)}`) : null,
        keepSecs ? el("span", {}, el("i", { class: "sw keep" }), `Keep ${fmtT(keepSecs, false)}`) : null,
        el("span", {}, el("i", { class: "sw cut" }), "Cuts"),
      ].filter(Boolean));

      if (document.activeElement !== this.inInput) this.inInput.value = this.sel.in != null ? fmtT(this.sel.in) : "";
      if (document.activeElement !== this.outInput) this.outInput.value = this.sel.out != null ? fmtT(this.sel.out) : "";
      const nKeep = (model.keep || []).length;
      this.keepBtn.hidden = !nKeep;
      this.keepBtn.replaceChildren(icon("scissors"), `Use keep ranges (${nKeep})`);
      const unsaved = model.cuts.filter((k) => k.state === "new").length;
      this.saveBtn.hidden = !this.opts.save || !unsaved;
      this.saveBtn.textContent = unsaved === 1 ? "Save 1 cut" : `Save ${unsaved} cuts`;

      this.list.replaceChildren(...model.cuts.map((k, i) => el("li", { class: `cut-row ${k.state}` },
        el("span", { class: "n mono", text: String(i + 1) }),
        el("button", { type: "button", class: "link mono", title: "Go to this cut", onclick: () => { this.sel = { in: k.start, out: k.end }; this.seek(k.start); this.render(this.model); } }, `${fmtT(k.start)} – ${fmtT(k.end)}`),
        el("span", { class: "muted", text: `${(k.end - k.start).toFixed(1)} s` }),
        el("span", { class: "file mono", text: k.file ? k.file.split("/").pop() : "" }),
        k.state === "saved" ? el("span", { class: "ok" }, icon("check-circle"), "Saved")
          : k.state === "failed" ? el("span", { class: "error", title: k.error || "" }, icon("close-circle"), "Failed")
          : el("span", { class: "muted", text: "Not saved yet" }),
        k.agent ? el("span", { class: "badge agent", text: "agent" }) : null,
        el("button", { type: "button", class: "icon small", "aria-label": `Remove cut ${i + 1}`, title: "Remove", onclick: () => this.removeCut(i) }, icon("close")))));
      if (!model.cuts.length) this.list.append(el("li", { class: "muted empty", text: "No cuts." }));

      this.offsetRow.hidden = !model.hasLog;
      if (document.activeElement !== this.offset) this.offset.value = model.logOffset ?? 0;
      this.note.textContent = model.logInterval > 0.3 ? `Radio log rows are ${model.logInterval.toFixed(1)} s apart. Moment times are within about ${model.logInterval.toFixed(1)} s.` : "";
      this.note.hidden = !this.note.textContent;
      this.renderMoments();
    }

    renderMoments() {
      const box = this.opts.momentsEl;
      if (!box || !this.model) return;
      const ms = this.model.moments || [];
      box.replaceChildren(...(ms.length ? ms.map((m) => el("li", {},
        icon(KIND_ICON[m.kind]),
        el("button", { type: "button", class: "link", onclick: () => this.frame(m) }, KIND[m.kind]),
        el("span", { class: "mono muted", text: fmtT(m.start, false) }),
        el("span", { class: "score", title: `Score ${m.score.toFixed(2)}` }, el("span", { style: `width:${Math.round(m.score * 100)}%` })),
        el("button", { type: "button", class: "ghost small", onclick: () => { this.frame(m); this.addCut(); } }, icon("scissors"), "Cut"))) : [el("li", { class: "muted", text: "No moments." })]));
      box.querySelectorAll("li").forEach((li, i) => ms[i] && li.classList.add(`k-${ms[i].kind}`));
    }

    drag(e, which) {
      e.preventDefault();
      const r = this.tl.getBoundingClientRect();
      const move = (ev) => {
        const t = ((ev.clientX - r.left) / r.width) * this.model.duration;
        const v = Math.max(0, Math.min(this.model.duration, Math.round(t * 10) / 10));
        if (which === "in") this.sel.in = Math.min(v, (this.sel.out ?? v) - 0.1);
        else this.sel.out = Math.max(v, (this.sel.in ?? v) + 0.1);
        this.seek(this.sel[which]);
        this.render(this.model);
      };
      const up = () => {
        window.removeEventListener("pointermove", move);
        window.removeEventListener("pointerup", up);
      };
      window.addEventListener("pointermove", move);
      window.addEventListener("pointerup", up);
    }

    frame(m) {
      const pad = m.kind === "dead_air" ? 0 : 1;
      this.sel = { in: +Math.max(0, m.start - pad).toFixed(1), out: +Math.min(this.model.duration, m.end + pad).toFixed(1) };
      this.seek(this.sel.in);
      this.render(this.model);
    }

    setPoint(which) {
      const t = this.playhead();
      if (t == null) return this.opts.toast("Play the clip first, or type the time.", true);
      this.sel[which] = +t.toFixed(1);
      this.render(this.model);
    }

    async addCut() {
      const a = this.sel.in;
      const b = this.sel.out;
      if (a == null || b == null || b - a < 0.5) return this.opts.toast("Set an in and an out point at least 0.5 s apart.", true);
      const cuts = [...this.model.cuts.map(({ start, end }) => ({ start, end })), { start: a, end: b }];
      this.sel = { in: null, out: null };
      await this.opts.setCuts(cuts);
    }

    async removeCut(i) {
      await this.opts.setCuts(this.model.cuts.filter((_, j) => j !== i).map(({ start, end }) => ({ start, end })));
    }

    async useKeep() {
      const keep = this.model.keep || [];
      const have = this.model.cuts.map(({ start, end }) => ({ start, end }));
      const fresh = keep.filter((k) => !have.some((c) => Math.abs(c.start - k.start) < 0.05 && Math.abs(c.end - k.end) < 0.05));
      await this.opts.setCuts([...have, ...fresh]);
    }

    armHere() {
      const t = this.playhead();
      if (t == null) return this.opts.toast("Play the clip to where you armed first.", true);
      this.opts.setOffset(+t.toFixed(1));
    }

    // Keys for the editor. Returns true when it handled the key.
    handleKey(e) {
      if (!this.model || this.root.hidden) return false;
      // Command, Control and Option shortcuts belong to the app (Command-I is Edit details).
      if (e.metaKey || e.ctrlKey || e.altKey) return false;
      const v = this.video;
      const k = e.key.length === 1 ? e.key.toLowerCase() : e.key;
      switch (k) {
        case " ":
          if (v && v.src) v.paused ? v.play() : v.pause();
          else return false;
          return true;
        case "j":
          if (v && v.src) v.currentTime = Math.max(0, v.currentTime - 5);
          return true;
        case "k":
          if (v) { v.pause(); v.playbackRate = 1; }
          return true;
        case "l":
          if (v && v.src) { if (v.paused) v.play(); else v.playbackRate = Math.min(4, v.playbackRate * 2); }
          return true;
        case "ArrowLeft":
        case "ArrowRight":
          if (!v || !v.src) return false;
          v.pause();
          v.currentTime = Math.max(0, v.currentTime + (k === "ArrowLeft" ? -FRAME : FRAME));
          return true;
        case "i":
          this.setPoint("in");
          return true;
        case "o":
          this.setPoint("out");
          return true;
        case "c":
          this.addCut();
          return true;
        default:
          return false;
      }
    }
  }

  return { Editor, apply, askRemoved, fmtT, parseT, KIND, KIND_ICON };
})();
