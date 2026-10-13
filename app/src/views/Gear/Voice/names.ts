// Readable names for voices and models: a voice's short name, a model's name from the account
// (or from its id), and a voice name from the slug of an older sample folder.
import type { ModelInfo, VoiceInfo } from "../../../ipc/types";

/** The voice's name before " - ": "Xavier - Dominating, Metalic Announcer" is "Xavier". */
export const shortName = (name: string) => name.split(" - ")[0].trim() || name;

/** The core's slug of a name: lower case, each other character a hyphen, no hyphen at the ends. */
export const slug = (s: string) =>
  s
    .replace(/[^A-Za-z0-9]/g, "-")
    .toLowerCase()
    .replace(/^-+|-+$/g, "");

/** A model's name: the account's, else one made from its id ("eleven_flash_v2_5" is
 *  "Eleven Flash v2.5"). */
export function modelName(id: string, models?: ModelInfo[] | null) {
  const m = models?.find((x) => x.id === id);
  if (m?.name) return m.name;
  if (!id.startsWith("eleven_")) return id;
  const words = id
    .slice("eleven_".length)
    .replace(/v(\d+)_(\d+)/g, "v$1.$2")
    .split("_")
    .map((w) => (/^v\d/.test(w) ? w : w.charAt(0).toUpperCase() + w.slice(1)));
  return ["Eleven", ...words].join(" ");
}

/** A sample card's voice name: the account's own spelling when the card's name is the same
 *  name with the punctuation lost (a folder from 0.19 and earlier). */
export function voiceName(name: string, voice: string, voices?: VoiceInfo[] | null) {
  const v = voices?.find((x) => (voice ? x.id === voice : slug(x.name) === slug(name)));
  return v?.name ?? name;
}
