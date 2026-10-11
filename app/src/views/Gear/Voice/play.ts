// Plays a sound file the core made or copied into the cache.
import { fileSrc } from "../../../ipc/api";

export function playFile(path: string) {
  const a = new Audio(fileSrc(path));
  void a.play().catch(() => undefined);
}
