// Background work the core reports: thumbnails, cuts, rebuild, copy, check, export. The
// sidebar footer shows each running one. Also the volumes and card counts.
import type { StateCreator } from "zustand";
import type { State } from ".";
import { api } from "../ipc/api";
import type { CardStatus, EnvCheck, Volume } from "../ipc/types";

export interface TasksSlice {
  tasks: Record<string, { done: number; total: number }>;
  volumes: Volume[];
  cards: Record<string, CardStatus>;
  env: EnvCheck | null;
  setTask: (task: string, done: number, total: number) => void;
  endTask: (task: string) => void;
  refreshVolumes: () => Promise<void>;
}

export const createTasksSlice: StateCreator<State, [], [], TasksSlice> = (set, get) => ({
  tasks: {},
  volumes: [],
  cards: {},
  env: null,
  setTask: (task, done, total) => set((s) => ({ tasks: { ...s.tasks, [task]: { done, total } } })),
  endTask: (task) =>
    set((s) => {
      const tasks = { ...s.tasks };
      delete tasks[task];
      return { tasks };
    }),
  refreshVolumes: async () => {
    const volumes = await api.listVolumes();
    const mounts = new Set(volumes.map((v) => v.mount));
    set({ volumes, cards: Object.fromEntries(Object.entries(get().cards).filter(([m]) => mounts.has(m))) });
    for (const v of volumes.filter((x) => x.is_card)) {
      api
        .cardStatus(v.mount)
        .then((st) => set((s) => ({ cards: { ...s.cards, [v.mount]: st } })))
        .catch(() => {});
    }
  },
});

export const TASK_LABEL: Record<string, string> = {
  thumbnails: "Making thumbnails",
  cuts: "Saving cuts",
  moments: "Finding dead air",
  rebuild: "Reading the library",
  stage: "Copying clips",
  analyse: "Checking clips",
  export: "Adding to library",
};
