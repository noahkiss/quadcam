// Plays a sound file the core made or copied into the cache.
import { api, fileSrc } from "../../../ipc/api";

export function playFile(path: string) {
  const a = new Audio(fileSrc(path));
  void a.play().catch(() => undefined);
}

/** Plays a voice's preview sample (its `preview_url`): the core fetches it into the cache
 *  once, so the page plays a local file. */
export async function playPreview(url: string) {
  playFile(await api.gearVoiceLibraryPreview(url));
}
