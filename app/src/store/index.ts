// One store in slices (plan 3.1). The core stays the source of truth: slices cache its
// library, session and settings, and its events trigger a refetch (see ../events.ts).
import type { ReactNode } from "react";
import { create } from "zustand";
import { createHistorySlice, type HistorySlice } from "./history";
import { createLibrarySlice, type LibrarySlice } from "./library";
import { createSessionSlice, type SessionSlice } from "./session";
import { createSettingsSlice, type SettingsSlice } from "./settings";
import { createTasksSlice, type TasksSlice } from "./tasks";
import { createUiSlice, type UiSlice } from "./ui";

export type State = LibrarySlice & SessionSlice & SettingsSlice & TasksSlice & UiSlice & HistorySlice;

export const useStore = create<State>()((...a) => ({
  ...createLibrarySlice(...a),
  ...createSessionSlice(...a),
  ...createSettingsSlice(...a),
  ...createTasksSlice(...a),
  ...createUiSlice(...a),
  ...createHistorySlice(...a),
}));

/** The store outside React. */
export const store = useStore;

/** Asks a question in a dialog. Resolves the text (or true without a field), or null. */
export function ask(title: string, text?: string, opts: { input?: string; ok?: string; danger?: boolean; body?: ReactNode } = {}) {
  return new Promise<string | true | null>((resolve) => {
    useStore.setState({
      askReq: {
        title,
        text,
        ...opts,
        resolve: (v) => {
          useStore.setState({ askReq: null });
          resolve(v);
        },
      },
    });
  });
}

/** Asks what happens to exported files whose cut is removed. */
export function askRemoved(files: string[]) {
  return new Promise<"keep" | "trash" | null>((resolve) => {
    useStore.setState({
      removedReq: {
        files,
        resolve: (v) => {
          useStore.setState({ removedReq: null });
          resolve(v);
        },
      },
    });
  });
}
