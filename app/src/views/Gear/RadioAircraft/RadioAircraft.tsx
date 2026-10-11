// A radio's "Aircraft on this radio": every aircraft profile that names the radio, each with
// its EdgeTX model and whether that model is on the card or in the latest backup. The profile
// is the one source of truth (`gear.radio`), so Add and Remove save the profile, and
// Settings > Aircraft edits the same field.
import { useState } from "react";
import { useStore } from "../../../store";
import { sel } from "../../../store/settings";
import { api, errText } from "../../../ipc/api";
import { Button } from "../../../components/Button";
import { Chip } from "../../../components/Chip";
import { Select } from "../../../components/Field";
import { toast } from "../../../components/toastStore";
import { deviceName } from "../../../lib/gear";
import { addable, aircraftOf, modelStatus, modelText, withRadio } from "../../../lib/radioAircraft";
import type { DeviceRef } from "../slots";
import styles from "./RadioAircraft.module.css";

export function RadioAircraft({ d }: { d: DeviceRef }) {
  const profiles = useStore(sel.profiles);
  const saved = useStore((s) => s.devices);
  const setFilter = useStore((s) => s.setFilter);
  const [pick, setPick] = useState("");
  const [busy, setBusy] = useState(false);
  const radio = d.device?.id;
  if (!radio) return null;
  const list = aircraftOf(d.device, saved);
  const options = addable(profiles, radio);
  const radioName = (id: string) => {
    const x = saved.find((r) => r.id === id);
    return x ? deviceName(x) : "another radio";
  };

  const save = async (name: string, on: boolean) => {
    const p = profiles.find((x) => x.name === name);
    if (!p) return;
    setBusy(true);
    try {
      await api.profileGear(p.name, withRadio(p, on ? radio : null));
      if (on) setPick("");
    } catch (e) {
      toast(errText(e), true);
    } finally {
      setBusy(false);
    }
  };

  return (
    <section className={styles.section} aria-labelledby="radio-aircraft">
      <h3 id="radio-aircraft">Aircraft on this radio</h3>
      {list.length === 0 ? (
        <p className={styles.empty}>No aircraft yet.</p>
      ) : (
        <table className={styles.table}>
          <thead>
            <tr>
              <th>Aircraft</th>
              <th>EdgeTX model</th>
              <th>On the radio</th>
              <th>
                <span className="visually-hidden">Actions</span>
              </th>
            </tr>
          </thead>
          <tbody>
            {list.map((a) => (
              <tr key={a.profile}>
                <td>
                  <span className={styles.name}>
                    {a.profile}
                    {a.selected && <Chip kind="accent">Selected</Chip>}
                  </span>
                </td>
                <td className="selectable">{modelText(a)}</td>
                <td>{modelStatus(a)}</td>
                <td className={styles.actions}>
                  <Button size="sm" variant="ghost" onClick={() => setFilter({ group: "all", aircraft: a.profile })}>
                    Show clips
                  </Button>
                  <Button size="sm" variant="danger-ghost" disabled={busy} aria-label={`Remove ${a.profile}`} onClick={() => void save(a.profile, false)}>
                    Remove
                  </Button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      {options.length > 0 && (
        <div className={styles.add}>
          <Select aria-label="Aircraft to add" value={pick} onChange={(e) => setPick(e.target.value)}>
            <option value="">Choose an aircraft</option>
            {options.map((p) => (
              <option key={p.name} value={p.name}>
                {p.gear?.radio ? `${p.name} (now on ${radioName(p.gear.radio)})` : p.name}
              </option>
            ))}
          </Select>
          <Button size="sm" disabled={!pick || busy} onClick={() => void save(pick, true)}>
            Add aircraft
          </Button>
        </div>
      )}
    </section>
  );
}
