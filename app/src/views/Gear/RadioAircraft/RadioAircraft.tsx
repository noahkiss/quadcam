// A radio's "Aircraft on this radio": every aircraft profile that names the radio, each with
// its EdgeTX model and whether that model is on the card or in the latest backup. The profile
// is the one source of truth (`gear.radio`), so Add and Remove save the profile, and
// Settings > Aircraft edits the same field.
import { useState } from "react";
import { useStore } from "../../../store";
import { sel } from "../../../store/settings";
import { api, errText } from "../../../ipc/api";
import { Button } from "../../../components/Button";
import { Badge } from "../../../components/dash/Badge";
import { DataTable, type Column } from "../../../components/dash/DataTable";
import { EmptyState } from "../../../components/dash/EmptyState";
import { SectionCard } from "../../../components/dash/SectionCard";
import { Select } from "../../../components/Field";
import { toast } from "../../../components/toastStore";
import { deviceName } from "../../../lib/gear";
import { addable, aircraftOf, modelStatus, modelText, withRadio } from "../../../lib/radioAircraft";
import type { RadioAircraft as RadioAircraftRow } from "../../../ipc/types";
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

  const columns: Column<RadioAircraftRow>[] = [
    {
      key: "aircraft",
      header: "Aircraft",
      rowHeader: true,
      cell: (a) => (
        <span className={styles.name}>
          {a.profile}
          {a.selected && <Badge tone="accent">Selected</Badge>}
        </span>
      ),
    },
    { key: "model", header: "EdgeTX model", cell: (a) => <span className="selectable">{modelText(a)}</span> },
    { key: "status", header: "On the radio", optional: true, cell: (a) => modelStatus(a) },
    {
      key: "actions",
      header: <span className="visually-hidden">Actions</span>,
      cell: (a) => (
        <div className={styles.actions}>
          <Button size="sm" variant="ghost" onClick={() => setFilter({ group: "all", aircraft: a.profile })}>
            Show Clips
          </Button>
          <Button size="sm" variant="danger-ghost" disabled={busy} aria-label={`Remove ${a.profile}`} onClick={() => void save(a.profile, false)}>
            Remove
          </Button>
        </div>
      ),
    },
  ];

  return (
    <SectionCard
      title="Aircraft on this radio"
      actions={
        options.length > 0 && (
          <>
            <Select aria-label="Aircraft to add" value={pick} onChange={(e) => setPick(e.target.value)}>
              <option value="">Choose an aircraft</option>
              {options.map((p) => (
                <option key={p.name} value={p.name}>
                  {p.gear?.radio ? `${p.name} (now on ${radioName(p.gear.radio)})` : p.name}
                </option>
              ))}
            </Select>
            <Button size="sm" disabled={!pick || busy} onClick={() => void save(pick, true)}>
              Add Aircraft
            </Button>
          </>
        )
      }
    >
      <DataTable label="Aircraft on this radio" columns={columns} rows={list} rowKey={(a) => a.profile} empty={<EmptyState icon="list">No aircraft yet.</EmptyState>} />
    </SectionCard>
  );
}
