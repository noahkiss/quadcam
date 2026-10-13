// A per-machine on/off choice kept in the web view's local storage, such as whether a section
// is open. A page without storage starts from the default each time.
import { useState } from "react";

function read(key: string, fallback: boolean) {
  try {
    const v = localStorage.getItem(key);
    return v === null ? fallback : v === "true";
  } catch {
    return fallback;
  }
}

/** A remembered flag: its value and a setter that saves it. */
export function useStoredFlag(key: string, fallback: boolean): [boolean, (on: boolean) => void] {
  const [on, setOn] = useState(() => read(key, fallback));
  const set = (v: boolean) => {
    setOn(v);
    try {
      localStorage.setItem(key, String(v));
    } catch {
      // Not kept: the next page opens with the default.
    }
  };
  return [on, set];
}
