// The core's `secret::hint`, for the mock: the first and last characters (4 from 16
// characters on, else a quarter) and the length, never more than half the value.
import type { Hint } from "../types";

export function hint(secret: string): Hint {
  const chars = [...secret];
  const n = chars.length >= 16 ? 4 : Math.floor(chars.length / 4);
  const prefix = chars.slice(0, n).join("");
  const suffix = n ? chars.slice(-n).join("") : "";
  return { hint: n ? `${prefix}…${suffix} (${chars.length} chars)` : `(${chars.length} chars)`, prefix, suffix, length: chars.length };
}
