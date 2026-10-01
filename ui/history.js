// Undo and redo for library edits. Each step holds its own undo and redo calls to the core;
// app.js builds them where the edit happens. A new edit clears the redo list.
"use strict";

const History = (() => {
  const done = [];
  const undone = [];
  const LIMIT = 100;
  let listener = () => {};

  // step: { label, undo: async () => {}, redo: async () => {} }
  function push(step) {
    done.push(step);
    if (done.length > LIMIT) done.shift();
    undone.length = 0;
    listener();
  }

  async function run(from, to, which) {
    const step = from.pop();
    if (!step) return null;
    try {
      await step[which]();
      to.push(step);
    } catch (e) {
      // The step no longer applies (the file moved, or left the Trash): drop it.
      listener();
      throw e;
    }
    listener();
    return step;
  }

  return {
    push,
    undo: () => run(done, undone, "undo"),
    redo: () => run(undone, done, "redo"),
    canUndo: () => done.length > 0,
    canRedo: () => undone.length > 0,
    undoLabel: () => done.at(-1)?.label || "",
    redoLabel: () => undone.at(-1)?.label || "",
    onChange: (fn) => { listener = fn; },
  };
})();
