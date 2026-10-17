// Number and date formats, as the app shows them.
import type { SourceKind } from "../ipc/types";

/** The video system of a card or an import, as a short label. */
export const SOURCE_LABEL: Record<SourceKind, string> = { analog: "Analog", dji: "DJI" };

export function fmtBytes(n: number): string {
  if (!n) return "0 B";
  const u = ["B", "KB", "MB", "GB", "TB"];
  const i = Math.min(u.length - 1, Math.floor(Math.log10(n) / 3));
  return `${(n / 10 ** (3 * i)).toFixed(i > 2 ? 1 : 0)} ${u[i]}`;
}

/** `m:ss`, or `h:mm:ss` from an hour. */
export function fmtDur(s: number): string {
  if (!s || s <= 0) return "0:00";
  s = Math.round(s);
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const ss = String(s % 60).padStart(2, "0");
  return h ? `${h}:${String(m).padStart(2, "0")}:${ss}` : `${m}:${ss}`;
}

/** `12 min`, `1 h 5 min`. */
export function fmtLong(s: number): string {
  const m = Math.round((s || 0) / 60);
  return m >= 60 ? `${Math.floor(m / 60)} h ${m % 60} min` : `${m} min`;
}

/** `Sun, Sep 27`, or `Sunday, Sep 27, 2026` when long. */
export function fmtDay(d: string, long = false): string {
  const dt = new Date(`${d}T12:00:00`);
  return dt.toLocaleDateString(undefined, long ? { weekday: "long", month: "short", day: "numeric", year: "numeric" } : { weekday: "short", month: "short", day: "numeric" });
}

/** Trim times: `1:02.5`, or `1:02` without tenths. */
export function fmtT(s: number, tenths = true): string {
  s = Math.max(0, s || 0);
  const m = Math.floor(s / 60);
  const rest = s - m * 60;
  return tenths ? `${m}:${rest.toFixed(1).padStart(4, "0")}` : `${m}:${String(Math.floor(rest)).padStart(2, "0")}`;
}

/** `1:02.5`, `62.5` or empty -> seconds or null. */
export function parseT(text: string): number | null {
  const t = String(text || "").trim();
  if (!t) return null;
  const m = t.match(/^(\d+):(\d+(?:\.\d+)?)$/);
  if (m) return +m[1] * 60 + +m[2];
  const n = Number(t);
  return Number.isFinite(n) ? n : null;
}

export const plural = (n: number, one: string, many = `${one}s`) => `${n} ${n === 1 ? one : many}`;

export const base = (p: string | null | undefined) => (p || "").split("/").pop() || "";

/** `/Users/me/Movies/x` -> `~/Movies/x` when `home` is known; `/Users/meg` stays as it is. */
export function tilde(p: string | null | undefined, home: string): string {
  const h = home.replace(/\/+$/, "");
  if (!p || !h) return p || "";
  return p === h || p.startsWith(h + "/") ? "~" + p.slice(h.length) : p;
}

/** A path as shown (`tilde`), cut before its last segment: `head` may be cut short in the
 *  middle of the line, `tail` (`/quadcam`) always shows. */
export function splitPath(p: string | null | undefined, home: string): { head: string; tail: string } {
  const shown = tilde(p, home);
  const end = shown.length > 1 && shown.endsWith("/") ? shown.length - 1 : shown.length;
  const cut = shown.lastIndexOf("/", end - 1);
  return cut > 0 ? { head: shown.slice(0, cut), tail: shown.slice(cut) } : { head: "", tail: shown };
}

/** How long ago `t` was, short: `Now`, `5 min`, `3 h`, `6 d`, `5 wk`; `—` without a time. */
export function fmtAgo(t: string | null | undefined, now = Date.now()): string {
  const ms = t ? Date.parse(t) : NaN;
  if (!Number.isFinite(ms)) return "—";
  const m = Math.max(0, Math.floor((now - ms) / 60000));
  if (m < 1) return "Now";
  if (m < 60) return `${m} min`;
  if (m < 60 * 24) return `${Math.floor(m / 60)} h`;
  const d = Math.floor(m / (60 * 24));
  return d < 28 ? `${d} d` : `${Math.floor(d / 7)} wk`;
}
