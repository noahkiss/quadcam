// Settings > Modules: tools QuadCam downloads from their upstream on request (ffmpeg,
// esptool). Install and Update show the license first; nothing downloads before Download.
import { useEffect, useState } from "react";
import { ask, useStore } from "../../store";
import { api, errText, openLink } from "../../ipc/api";
import type { ModulePin, ModuleStatus } from "../../ipc/types";
import { Button } from "../../components/Button";
import { SelectField } from "../../components/Field";
import { toast } from "../../components/toastStore";
import { loadEnv } from "../../events";
import type { Draft } from "./draft";
import styles from "./SettingsSheet.module.css";

const mb = (bytes: number) => `${(bytes / 1e6).toFixed(1)} MB`;

function Link({ href, children }: { href: string; children: string }) {
  return (
    <a
      href={href}
      onClick={(e) => {
        e.preventDefault();
        openLink(href).catch(() => {});
      }}
    >
      {children}
    </a>
  );
}

/** The license prompt: name, version, size, license, source and the download's origin. */
function Prompt({ pin }: { pin: ModulePin }) {
  return (
    <dl className={styles.prompt}>
      <dt>Version</dt>
      <dd>{pin.version}</dd>
      <dt>Size</dt>
      <dd>{mb(pin.assets.reduce((n, a) => n + a.size, 0))}</dd>
      <dt>License</dt>
      <dd>
        <Link href={pin.license_url}>{pin.license}</Link>
      </dd>
      <dt>Source</dt>
      <dd>
        <Link href={pin.source}>{pin.source}</Link>
      </dd>
      <dt>Download from</dt>
      <dd className="mono selectable">{pin.assets.map((a) => a.url).join("\n")}</dd>
    </dl>
  );
}

export function ModulesPane({ d, set }: { d: Draft; set: <K extends keyof Draft>(k: K, v: Draft[K]) => void }) {
  const [list, setList] = useState<ModuleStatus[] | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [note, setNote] = useState<string | null>(null);
  const ffmpeg = useStore((s) => s.env?.tools?.ffmpeg);

  useEffect(() => {
    api.modules().then(setList, (e) => setNote(errText(e)));
  }, []);

  const replace = (m: ModuleStatus) => setList((l) => (l || []).map((x) => (x.name === m.name ? m : x)));

  const install = async (m: ModuleStatus) => {
    const pin = m.newest ?? m.pinned;
    const yes = await ask(`${m.installed ? "Update" : "Install"} ${pin.title}?`, `QuadCam downloads ${pin.title} from its upstream and checks its checksum.`, { ok: "Download", body: <Prompt pin={pin} /> });
    if (yes !== true) return;
    setBusy(m.name);
    try {
      replace(await api.moduleInstall(m.name));
      toast(`${pin.title} ${pin.version} is installed.`);
      await loadEnv();
    } catch (e) {
      toast(errText(e), true);
    } finally {
      setBusy(null);
    }
  };

  const remove = async (m: ModuleStatus) => {
    const yes = await ask(`Remove ${m.pinned.title}?`, "QuadCam deletes the module's folder. A feature that needs it asks again.", { ok: "Remove", danger: true });
    if (yes !== true) return;
    setBusy(m.name);
    try {
      replace(await api.moduleRemove(m.name));
      await loadEnv();
    } catch (e) {
      toast(errText(e), true);
    } finally {
      setBusy(null);
    }
  };

  const check = async () => {
    setBusy("check");
    setNote(null);
    try {
      const l = await api.modulesCheck();
      setList(l);
      setNote(l.some((m) => m.update) ? "Updates are available." : "Every installed module is up to date.");
    } catch (e) {
      setNote(errText(e));
    } finally {
      setBusy(null);
    }
  };

  return (
    <section>
      <h3>Modules</h3>
      <p className={styles.sub}>Tools QuadCam downloads from their upstream when you install them.</p>
      <table className={styles.modules}>
        <thead>
          <tr>
            <th scope="col">Module</th>
            <th scope="col">Version</th>
            <th scope="col">Size</th>
            <th scope="col">License</th>
            <th scope="col">Source</th>
            <th scope="col">
              <span className="visually-hidden">Actions</span>
            </th>
          </tr>
        </thead>
        <tbody>
          {(list || []).map((m) => {
            const pin = m.newest ?? m.pinned;
            const shown = m.installed ?? pin;
            return (
              <tr key={m.name}>
                <th scope="row">
                  <b>{m.pinned.title}</b>
                  <span className={styles.sub}>{m.problem || m.pinned.about}</span>
                </th>
                <td>{m.installed ? m.installed.version : <span className={styles.sub}>Not installed</span>}</td>
                <td>{m.installed ? mb(m.installed.size) : mb(pin.assets.reduce((n, a) => n + a.size, 0))}</td>
                <td>
                  <Link href={shown.license_url}>{shown.license}</Link>
                </td>
                <td>
                  <Link href={shown.source}>Source</Link>
                </td>
                <td className={styles.actionsCell}>
                  {!m.installed && (
                    <Button size="sm" disabled={!!busy} onClick={() => install(m)} aria-label={`Install ${m.pinned.title}`}>
                      {busy === m.name ? "Installing…" : "Install"}
                    </Button>
                  )}
                  {m.installed && (m.update || m.problem) && (
                    <Button size="sm" disabled={!!busy} onClick={() => install(m)} aria-label={`${m.problem ? "Reinstall" : "Update"} ${m.pinned.title}`}>
                      {busy === m.name ? "Installing…" : m.problem ? "Reinstall" : `Update to ${pin.version}`}
                    </Button>
                  )}
                  {m.installed && (
                    <Button size="sm" variant="danger-ghost" disabled={!!busy} onClick={() => remove(m)} aria-label={`Remove ${m.pinned.title}`}>
                      Remove
                    </Button>
                  )}
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>
      <div className={styles.row}>
        <Button size="sm" variant="ghost" icon="refresh" disabled={!!busy} onClick={check}>
          {busy === "check" ? "Checking…" : "Check for updates"}
        </Button>
        {note && <span className={styles.sub} role="status">{note}</span>}
      </div>
      <h3>ffmpeg</h3>
      <SelectField label="Use ffmpeg from" value={d.ffmpegSource} onChange={(e) => set("ffmpegSource", e.target.value as Draft["ffmpegSource"])}>
        <option value="module">The QuadCam module, else Homebrew</option>
        <option value="homebrew">Homebrew</option>
      </SelectField>
      {ffmpeg && <p className={`${styles.sub} mono selectable`}>{`In use: ${ffmpeg}`}</p>}
    </section>
  );
}
