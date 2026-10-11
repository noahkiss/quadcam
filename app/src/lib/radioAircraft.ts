// A radio's aircraft (`Device.radio_aircraft`): the profiles that name the radio. The profile
// is the one source of truth, so adding or removing an aircraft here saves its `gear.radio`.
import type { Device, Profile, RadioAircraft } from "../ipc/types";

/** Where the aircraft's model is, as the core last looked. */
export function modelStatus(a: Pick<RadioAircraft, "model" | "checked" | "found">): string {
  const where = a.checked === "card" ? "on the card" : `in the ${a.checked}`;
  if (!a.model) return a.checked ? `No matching model ${where}` : "No model named";
  if (a.found == null || !a.checked) return "Not checked: no backup yet";
  return a.found ? where[0].toUpperCase() + where.slice(1) : `Not ${where}`;
}

/** The model as the list shows it: its name on the radio, then its file. */
export const modelText = (a: Pick<RadioAircraft, "model" | "model_name">) => [a.model_name, a.model].filter(Boolean).join(" · ") || "None";

/** The radio's aircraft: from the radio's own record, else the saved list. */
export function aircraftOf(d: Device | null | undefined, saved: Device[]): RadioAircraft[] {
  if (!d) return [];
  return d.radio_aircraft ?? saved.find((x) => x.id === d.id)?.radio_aircraft ?? [];
}

/** Profiles that can be added: every profile that does not name this radio yet. */
export const addable = (profiles: Profile[], radio: string) => profiles.filter((p) => p.gear?.radio !== radio);

/** The profile's gear with this radio set, or taken off. */
export function withRadio(p: Profile, radio: string | null): NonNullable<Profile["gear"]> {
  const g = { ...(p.gear ?? {}) };
  if (radio) g.radio = radio;
  else delete g.radio;
  return g;
}
