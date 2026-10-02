// Undo and redo for library edits. Each step holds its own undo and redo calls to the
// core; actions/library.ts builds them. A new edit clears redo, at most 100 steps are
// kept, and a step that fails is dropped.
import type { StateCreator } from "zustand";
import type { State } from ".";

export interface Step {
  label: string;
  undo: () => Promise<unknown>;
  redo: () => Promise<unknown>;
}

export const HISTORY_LIMIT = 100;

export interface HistorySlice {
  done: Step[];
  undone: Step[];
  pushStep: (step: Step) => void;
  /** Runs the last step's undo (or redo). Resolves the step, or null when there is none. */
  undo: () => Promise<Step | null>;
  redo: () => Promise<Step | null>;
}

export const createHistorySlice: StateCreator<State, [], [], HistorySlice> = (set, get) => {
  const run = async (from: "done" | "undone", which: "undo" | "redo") => {
    const list = get()[from];
    const step = list.at(-1);
    if (!step) return null;
    set({ [from]: list.slice(0, -1) } as Partial<State>);
    // A failed step no longer applies (the file moved, or left the Trash): it stays dropped.
    await step[which]();
    const to = from === "done" ? "undone" : "done";
    set({ [to]: [...get()[to], step] } as Partial<State>);
    return step;
  };
  return {
    done: [],
    undone: [],
    pushStep: (step) => set((s) => ({ done: [...s.done, step].slice(-HISTORY_LIMIT), undone: [] })),
    undo: () => run("done", "undo"),
    redo: () => run("undone", "redo"),
  };
};
