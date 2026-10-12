// Plays a sound file the core made or copied into the cache.
import { api, fileSrc } from "../../../ipc/api";
import { toast } from "../../../components/toastStore";
import { playError } from "./usePlayer";

/** Plays one file; a file the web view cannot load or play shows a toast. */
export function playFile(path: string) {
  const a = new Audio(fileSrc(path));
  let told = false;
  const fail = () => {
    if (told) return;
    told = true;
    toast(playError(path.split("/").pop() || path), true);
  };
  a.onerror = fail;
  void a.play().catch(fail);
}

/** Plays a voice's preview sample (its `preview_url`): the core fetches it into the cache
 *  once, so the page plays a local file. */
export async function playPreview(url: string) {
  playFile(await api.gearVoiceLibraryPreview(url));
}
