// Copy from another radio (`docs/radio-design.md` 5.5): pick a saved radio and its models.
// The models are copied across with their controls mapped onto this radio's, and the other
// radio's saved setup comes along with its controls mapped. Both go into this radio's one
// "Radio setup" change; nothing is written until the apply sheet writes it.
import { useEffect, useState } from "react";
import { Button } from "../../../components/Button";
import { Dialog } from "../../../components/Dialog";
import { Checkbox, SelectField } from "../../../components/Field";
import { api, errText } from "../../../ipc/api";
import { deviceName } from "../../../lib/gear";
import { useStore } from "../../../store";
import type { UseSetup } from "./useSetup";
import styles from "./Setup.module.css";

export function CopyDialog({ device, s, close }: { device: string; s: UseSetup; close: () => void }) {
  const radios = useStore((st) => st.devices).filter((d) => d.kind === "radio" && d.id !== device);
  const [from, setFrom] = useState(radios.find((d) => d.last_backup)?.id ?? "");
  const [loaded, setLoaded] = useState<{ from: string; models: { file: string; name: string }[] } | null>(null);
  const [picked, setPicked] = useState<string[]>([]);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!from) return;
    let gone = false;
    api.gearModel({ device: from, model: null, staged: false }).then(
      (m) => {
        if (gone) return;
        const list = m.models.map((x) => ({ file: x.file, name: x.name || x.file }));
        setLoaded({ from, models: list });
        setPicked(list.map((x) => x.file));
        setError(null);
      },
      (e) => !gone && setError(errText(e)),
    );
    return () => {
      gone = true;
    };
  }, [from]);

  const models = loaded?.from === from ? loaded.models : null;
  const done = async (v: string) => {
    if (v !== "copy") return close();
    const c = await s.edit({ copy_from: { from, models: picked, links: [] } });
    if (c) close();
  };
  return (
    <Dialog
      open
      title="Copy from another radio"
      onClose={(v) => void done(v)}
      actions={
        <>
          <Button type="submit" value="cancel" variant="ghost">
            Cancel
          </Button>
          <Button type="submit" value="copy" variant="primary" disabled={!from || !picked.length || s.busy}>
            Copy
          </Button>
        </>
      }
    >
      <div className={styles.form}>
        <SelectField
          label="Copy from"
          value={from}
          onChange={(e) => setFrom(e.target.value)}
        >
          <option value="">Pick a radio</option>
          {radios.map((d) => (
            <option key={d.id} value={d.id} disabled={!d.last_backup}>
              {deviceName(d)}
              {d.last_backup ? "" : " (no backup)"}
            </option>
          ))}
        </SelectField>
        {models && (
          <fieldset className={styles.models}>
            <legend>Models</legend>
            {models.map((m) => (
              <Checkbox key={m.file} label={m.name} detail={m.file} checked={picked.includes(m.file)} onChange={(e) => setPicked(e.target.checked ? [...picked, m.file] : picked.filter((x) => x !== m.file))} />
            ))}
          </fieldset>
        )}
        {(error ?? s.error) && (
          <p className={styles.error} role="alert">
            {error ?? s.error}
          </p>
        )}
      </div>
    </Dialog>
  );
}
