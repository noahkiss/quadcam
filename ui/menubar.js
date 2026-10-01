// The native menu bar's side in the webview: runs the action of each item (the same
// functions the keys call) and keeps the items enabled and checked to match the screen.
// The menu itself is built in src-tauri/src/menu.rs.
"use strict";

const MenuBar = (() => {
  const typingIn = () => document.activeElement?.closest?.("input, textarea, select, [contenteditable]");
  const dialogOpen = () => !!document.querySelector("dialog[open]");
  // The clips a Clip menu item acts on: the open clip, else the selection.
  const targets = () => (state.screen === "detail" && state.detailId ? [state.detailId] : selectedIds());

  const run = {
    settings: () => openSettings("library"),
    import: () => actions.import(),
    "open-folder": () => actions["open-folder"](),
    "reveal-library": () => actions["reveal-library"](),
    undo: () => (typingIn() ? document.execCommand("undo") : undoRedo("undo")),
    redo: () => (typingIn() ? document.execCommand("redo") : undoRedo("redo")),
    "select-all": () => {
      const t = typingIn();
      if (t?.select) return t.select();
      if (state.screen === "library") selectAll();
    },
    pick: () => rate(targets(), null, "pick"),
    reject: () => rate(targets(), null, "reject"),
    unflag: () => rate(targets(), null, "none"),
    rename: () => startRename(targets()[0]),
    "edit-details": () => openDetail(targets()[0], null, "details"),
    "reveal-clip": () => T.opener.revealItemInDir(libClip(targets()[0]).file),
    share: () => shareClips(targets()),
    "add-photos": () => addLibToPhotos(targets()),
    trash: () => trashClips(targets()),
    "view-grid": () => actions["view-grid"](),
    "view-list": () => actions["view-list"](),
    "thumb-bigger": () => setThumbSize((settings.thumbSize || 3) + 1),
    "thumb-smaller": () => setThumbSize((settings.thumbSize || 3) - 1),
    "prev-clip": () => stepClip(-1),
    "next-clip": () => stepClip(1),
  };
  for (let i = 0; i <= 5; i++) run[`rate-${i}`] = () => rate(targets(), i);
  for (const k of ["date", "rating", "duration", "name"]) run[`sort-${k}`] = () => setSort(k);

  T.event.listen("menu", ({ payload: id }) => run[id]?.());

  let last = "";
  // Sends the items' state when it changed. Cheap enough to call after every render.
  function sync() {
    const free = !dialogOpen();
    const lib = free && state.screen === "library";
    const n = targets().length;
    const clipOn = free && (state.screen === "library" || state.screen === "detail") && n > 0;
    const enabled = {
      settings: free, import: free && !!state.tools, "open-folder": free && !!state.tools, "reveal-library": true,
      undo: !!typingIn() || (free && History.canUndo()), redo: !!typingIn() || (free && History.canRedo()),
      "select-all": !!typingIn() || lib,
      pick: clipOn, reject: clipOn, unflag: clipOn, share: clipOn, "add-photos": clipOn, trash: clipOn,
      rename: clipOn && n === 1, "edit-details": clipOn && n === 1, "reveal-clip": clipOn && n === 1,
      "view-grid": lib, "view-list": lib, "thumb-bigger": lib && settings.libView !== "list" && (settings.thumbSize || 3) < 5,
      "thumb-smaller": lib && settings.libView !== "list" && (settings.thumbSize || 3) > 1,
      "prev-clip": free && (state.screen === "detail" || (lib && visible().length > 0)),
      "next-clip": free && (state.screen === "detail" || (lib && visible().length > 0)),
    };
    for (let i = 0; i <= 5; i++) enabled[`rate-${i}`] = clipOn;
    const sortKey = librarySort().key;
    const checked = { "view-grid": settings.libView !== "list", "view-list": settings.libView === "list" };
    for (const k of ["date", "rating", "duration", "name"]) {
      enabled[`sort-${k}`] = lib;
      checked[`sort-${k}`] = sortKey === k;
    }
    const key = JSON.stringify([enabled, checked]);
    if (key === last) return;
    last = key;
    invoke("menu_state", { enabled, checked }).catch(() => {});
  }

  // Dialogs and focus change what applies without a render.
  for (const ev of ["focusin", "focusout"]) document.addEventListener(ev, () => queueMicrotask(sync));
  for (const d of document.querySelectorAll("dialog")) {
    d.addEventListener("close", () => setTimeout(sync));
    new MutationObserver(() => sync()).observe(d, { attributes: true, attributeFilter: ["open"] });
  }
  History.onChange(sync);

  return { sync };
})();
