// Device refs for the Gear pages and the status bar: each device once, by page key, with
// its state now.
import { useEffect, useState } from "react";
import type { State } from "../../store";
import { deviceState, pageKey, type StateInput } from "../../lib/gear";
import { gearSlots, type DeviceRef } from "./slots";

/** The state inputs from the store at `now`. */
export const stateInput = (s: State, now: number): StateInput => ({
  status: s.gear,
  unmountedSince: s.unmountedSince,
  graceS: s.gear?.settings.cues.reminder_grace_s ?? 60,
  now,
});

/** Plugged-in devices first (connected, then unmounted but still in), then saved ones that
 *  are not plugged in. */
export function deviceRefs(s: State, now: number): DeviceRef[] {
  const x = stateInput(s, now);
  const out = new Map<string, DeviceRef>();
  const add = (c: State["unmounted"][number], unmounted: boolean) => {
    const key = pageKey(c);
    if (out.has(key)) return;
    const saved = c.device || (c.id ? s.devices.find((d) => d.id === c.id) : undefined) || null;
    let state = deviceState({ ...c, device: saved }, unmounted, x);
    if (state === "connected" && gearSlots.attention(c)) state = "attention";
    out.set(key, { key, kind: c.kind, device: saved, connected: c, unmounted, state });
  };
  for (const c of s.gear?.connected || []) add(c, false);
  for (const c of s.unmounted) add(c, true);
  for (const d of s.devices) if (!out.has(d.id)) out.set(d.id, { key: d.id, kind: d.kind, device: d, connected: null, unmounted: false, state: null });
  return [...out.values()];
}

/** The refs that are plugged in. */
export const pluggedIn = (refs: DeviceRef[]) => refs.filter((r) => r.connected);

/** The time, ticking every few seconds while `active` (the "still inserted" state comes
 *  with time). */
export function useNow(active: boolean, everyMs = 5000) {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!active) return;
    const tick = () => setNow(Date.now());
    const first = setTimeout(tick, 0);
    const t = setInterval(tick, everyMs);
    return () => {
      clearTimeout(first);
      clearInterval(t);
    };
  }, [active, everyMs]);
  return now;
}
