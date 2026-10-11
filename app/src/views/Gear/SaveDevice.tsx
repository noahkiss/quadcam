import { useState } from "react";
import { useStore } from "../../store";
import { api, errText } from "../../ipc/api";
import { sel } from "../../store/settings";
import { Button } from "../../components/Button";
import { Dialog } from "../../components/Dialog";
import { SelectField, TextField } from "../../components/Field";
import { toast } from "../../components/toastStore";
import { KIND_LABEL } from "../../lib/gear";
import type { DeviceRef } from "./slots";
import styles from "./Gear.module.css";

/** Names a device and links it to an aircraft: the small sheet for a new device, and Edit.
 *  A radio flies many aircraft, so its Overview lists them instead. */
export function SaveDevice({ d, onClose }: { d: DeviceRef; onClose: () => void }) {
  const profiles = useStore(sel.profiles);
  const [name, setName] = useState(d.device?.name || "");
  const [aircraft, setAircraft] = useState(d.device?.aircraft || "");
  const id = d.device?.id || d.connected?.id || "";
  const isRadio = d.kind === "radio";
  const close = async (v: string) => {
    if (v === "save") {
      try {
        await api.gearDeviceSave(id, name.trim(), isRadio ? null : aircraft);
      } catch (e) {
        toast(errText(e), true);
      }
    }
    onClose();
  };
  return (
    <Dialog
      open
      title={d.device ? "Edit device" : `Save this ${KIND_LABEL[d.kind]}`}
      onClose={close}
      actions={
        <>
          <Button type="submit" value="cancel" variant="ghost">
            Cancel
          </Button>
          <Button type="submit" value="save" variant="primary">
            Save
          </Button>
        </>
      }
    >
      <div className={styles.form}>
        <TextField label="Name" placeholder={`Unnamed ${KIND_LABEL[d.kind]}`} value={name} onChange={(e) => setName(e.target.value)} autoFocus />
        {!isRadio && (
          <SelectField label="Aircraft" value={aircraft} onChange={(e) => setAircraft(e.target.value)}>
            <option value="">None</option>
            {profiles.map((p) => (
              <option key={p.name} value={p.name}>
                {p.name}
              </option>
            ))}
          </SelectField>
        )}
      </div>
    </Dialog>
  );
}
