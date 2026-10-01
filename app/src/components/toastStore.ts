import { create } from "zustand";

interface ToastState {
  message: string;
  error: boolean;
  /** Bumps on every toast, so the same text twice shows twice. */
  seq: number;
}

export const useToast = create<ToastState>(() => ({ message: "", error: false, seq: 0 }));

let timer: ReturnType<typeof setTimeout> | undefined;

/** Shows a short message at the bottom of the window. Errors stay longer. */
export function toast(message: string, error = false) {
  clearTimeout(timer);
  useToast.setState((s) => ({ message, error, seq: s.seq + 1 }));
  timer = setTimeout(() => useToast.setState({ message: "" }), error ? 6000 : 3500);
}
