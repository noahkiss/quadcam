// Installing a module from outside Settings: the "ffmpeg not found" banner asks the same
// question Settings > Modules does, and nothing downloads before the person selects Download.
import { ask } from "../store";
import { api, errText } from "../ipc/api";
import { toast } from "../components/toastStore";
import { ModulePrompt } from "../components/ModulePrompt";
import { loadEnv } from "../events";

/** Asks, then downloads and installs the named module. True when it is installed. */
export async function installModule(name: string): Promise<boolean> {
  try {
    const m = (await api.modules()).find((x) => x.name === name);
    if (!m) throw new Error(`No module named ${name}.`);
    const pin = m.newest ?? m.pinned;
    const yes = await ask(`Install ${pin.title}?`, `QuadCam downloads ${pin.title} from its upstream and checks its checksum.`, { ok: "Download", body: <ModulePrompt pin={pin} /> });
    if (yes !== true) return false;
    await api.moduleInstall(name);
    toast(`${pin.title} ${pin.version} is installed.`);
    await loadEnv();
    return true;
  } catch (e) {
    toast(errText(e), true);
    return false;
  }
}
