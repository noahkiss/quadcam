// The import session: the clips being imported, owned by the core. The sheet's own state
// (step, selected clip, progress) lives here too.
import type { StateCreator } from "zustand";
import type { State } from ".";
import type { CardRelease, Clip, ClipDeletion, ClipPlan, ClipResult, FormatPlan, Session, StageProgress } from "../ipc/types";

export type Step = "load" | "review" | "export" | "finish";

export interface SessionSlice {
  session: Session | null;
  importOpen: boolean;
  step: Step;
  selectedClip: number | null;
  /** A load or an export is running. */
  busy: boolean;
  staging: StageProgress | null;
  /** Clip id -> 0..1 while converting. */
  progress: Record<number, number>;
  rTab: "details" | "flight" | "file";
  /** An agent's erase request waiting for the person's click. */
  agentFormat: number | null;
  /** Where a load is copying from, for the sheet's first message. */
  loadingFrom: string | null;
  exportTotal: number;
  exportDone: number;
  photosStatus: string;
  formatPlan: FormatPlan | null;
  formatError: string | null;
  formatUnlocked: boolean;
  /** The volume name typed for the format step. */
  formatLabelDraft: string;
  /** This import keeps the clips on the card although "Delete clips after import" is on. */
  keepClips: boolean;
  /** What the last export deleted from the card, or null when it deleted nothing. */
  clipDeletion: ClipDeletion[] | null;
  /** What the end of the last export did with the card: unmounted, or why not. */
  cardRelease: CardRelease | null;
  setSession: (s: Session | null) => void;
  setStep: (step: Step) => void;
  setImportOpen: (open: boolean) => void;
  selectClip: (id: number | null) => void;
  setRTab: (t: "details" | "flight" | "file") => void;
}

export const createSessionSlice: StateCreator<State, [], [], SessionSlice> = (set, get) => ({
  session: null,
  importOpen: false,
  step: "review",
  selectedClip: null,
  busy: false,
  staging: null,
  progress: {},
  rTab: "details",
  agentFormat: null,
  loadingFrom: null,
  exportTotal: 0,
  exportDone: 0,
  photosStatus: "",
  formatPlan: null,
  formatError: null,
  formatUnlocked: false,
  formatLabelDraft: "",
  keepClips: false,
  clipDeletion: null,
  cardRelease: null,
  setSession: (session) => {
    const cur = get().selectedClip;
    const keep = session && cur != null && session.clips.some((c) => c.id === cur);
    // A different set of clips is a new import: its delete choice starts from the setting.
    const fresh = sessionKey(session) !== sessionKey(get().session);
    set({ session, selectedClip: keep ? cur : (session?.clips[0]?.id ?? null), ...(fresh ? { keepClips: false, clipDeletion: null, cardRelease: null } : {}) });
  },
  setStep: (step) => set({ step }),
  setImportOpen: (importOpen) => set({ importOpen }),
  selectClip: (selectedClip) => set({ selectedClip }),
  setRTab: (rTab) => set({ rTab }),
});

const sessionKey = (s: Session | null) => (s ? `${s.source}\n${s.clips.map((c) => c.card_path).join("\n")}` : "");

export const planOf = (s: Session | null, id: number): ClipPlan | undefined => s?.plans.find((p) => p.id === id);

/** The clips that import on their own: every clip but the later files of a joined recording. */
export const clipsOf = (s: Session | null): Clip[] => (s?.clips || []).filter((c) => c.part_of == null);

/** The DVR files a clip is made of: its own, then a joined recording's later files. */
export const filesOf = (s: Session, c: Clip): Clip[] => [c, ...(c.join?.on ? c.join.parts.map((id) => s.clips.find((x) => x.id === id)).filter((x): x is Clip => !!x) : [])];

/** The newest result for a clip. */
export const resultOf = (s: Session | null, id: number): ClipResult | undefined => [...(s?.results || [])].reverse().find((r) => r.id === id);

/** Clips not skipped and not yet verified. */
export function sessionLeft(s: Session | null): number {
  if (!s) return 0;
  const parts = new Set(s.clips.filter((c) => c.part_of != null).map((c) => c.id));
  return s.plans.filter((p) => !p.skip && !parts.has(p.id) && !s.results.some((r) => r.id === p.id && r.outcome === "verified")).length;
}

/** The session still has work: clips left, or nothing exported yet. */
export const unfinished = (s: Session | null) => !!s && (sessionLeft(s) > 0 || !s.results.length);
