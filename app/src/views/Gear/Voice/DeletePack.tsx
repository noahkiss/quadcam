// Delete a voice pack from this Mac. The raw takes stay in the cache unless the person ticks
// the box, so a new render of that voice and model costs nothing.
import { useState } from "react";
import { Button } from "../../../components/Button";
import { Dialog } from "../../../components/Dialog";
import { Checkbox } from "../../../components/Field";
import type { VoicePack } from "../../../ipc/types";
import { fmtBytes } from "../../../lib/format";
import styles from "./Voice.module.css";

/** `onClose(takes)`: null cancels; otherwise delete, with the raw takes when true. */
export function DeletePack({ pack, radios, onClose }: { pack: VoicePack; radios: string[]; onClose: (takes: boolean | null) => void }) {
  const [takes, setTakes] = useState(false);
  return (
    <Dialog
      open
      title={`Delete ${pack.voice}?`}
      onClose={(v) => onClose(v === "delete" ? takes : null)}
      actions={
        <>
          <Button type="submit" value="cancel" variant="ghost">
            Cancel
          </Button>
          <Button type="submit" value="delete" variant="danger">
            Delete
          </Button>
        </>
      }
    >
      <div className={styles.form}>
        <p className={styles.status}>
          Removes {pack.id} ({fmtBytes(pack.bytes)}) from this Mac. Radio cards keep the sounds they have.
          {radios.length > 0 ? ` Chosen for ${radios.join(", ")}: the delete clears that choice.` : ""}
        </p>
        <Checkbox label="Also delete the raw takes" detail="A new render of this voice and model then calls the provider again." checked={takes} onChange={(e) => setTakes(e.target.checked)} />
      </div>
    </Dialog>
  );
}
