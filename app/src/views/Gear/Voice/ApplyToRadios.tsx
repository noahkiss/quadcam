// Apply to radios…: a pack's "choose voice" change staged on each saved EdgeTX radio picked,
// the same change a radio's Voice segment stages. A radio whose card is in now can be
// reviewed and applied at once; the others apply when they are next plugged in.
import { useState } from "react";
import { Button } from "../../../components/Button";
import { Dialog } from "../../../components/Dialog";
import { Checkbox } from "../../../components/Field";
import { api, errText } from "../../../ipc/api";
import type { Device, VoiceChooseRadiosReport, VoicePack } from "../../../ipc/types";
import { deviceName } from "../../../lib/gear";
import { useStore } from "../../../store";
import { deviceRefs, useNow } from "../refs";
import styles from "./Voice.module.css";

/** A saved radio a pack can go on: EdgeTX, or a firmware not read yet. */
export const isEdgeTx = (d: Device) => d.kind === "radio" && (!d.identity?.firmware || d.identity.firmware.toLowerCase() === "edgetx");

/** The radio's card can be written now: mounted, or unmounted and still in. */
function useReachable() {
  const s = useStore();
  const refs = deviceRefs(s, useNow(false));
  return (id: string) => refs.some((r) => r.device?.id === id && r.connected && (r.unmounted || r.connected.link.kind === "volume"));
}

export function ApplyToRadios({ pack, onClose, onStaged }: { pack: VoicePack; onClose: () => void; onStaged: () => Promise<void> }) {
  const radios = useStore((s) => s.devices).filter(isEdgeTx);
  const loadGear = useStore((s) => s.loadGear);
  const openApply = useStore((s) => s.openApply);
  const reachable = useReachable();
  const [picked, setPicked] = useState<string[]>([]);
  const [keep, setKeep] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [done, setDone] = useState<VoiceChooseRadiosReport | null>(null);
  const all = radios.length > 0 && radios.every((d) => picked.includes(d.id));
  const name = (id: string) => {
    const d = radios.find((x) => x.id === id);
    return d ? deviceName(d) : id;
  };

  const close = async (v: string) => {
    if (v !== "stage") return onClose();
    try {
      const r = await api.gearVoiceChooseRadios({ pack: pack.id, radios: all ? [] : picked, all, keep_overrides: keep, editor: null });
      setError(null);
      setDone(r);
      await loadGear();
      await onStaged();
    } catch (e) {
      setError(errText(e));
    }
  };

  return (
    <Dialog
      open
      title={`Apply ${pack.voice} to radios`}
      onClose={close}
      actions={
        done ? (
          <Button type="submit" value="done" variant="primary">
            Done
          </Button>
        ) : (
          <>
            <Button type="submit" value="cancel" variant="ghost">
              Cancel
            </Button>
            <Button type="submit" value="stage" variant="primary" disabled={picked.length === 0}>
              Stage
            </Button>
          </>
        )
      }
    >
      <div className={styles.form}>
        {done ? (
          <ul className={styles.packs} aria-label="Staged">
            {done.staged.map((c) => (
              <li key={c.id} className={styles.pack}>
                <span className={styles.grow}>
                  <strong>{name(c.device)}</strong>: {reachable(c.device) ? "staged." : "staged. It applies when you plug the radio in."}
                </span>
                {reachable(c.device) && (
                  <Button
                    size="sm"
                    aria-label={`Review ${name(c.device)}`}
                    onClick={() => {
                      onClose();
                      void openApply(c.device, c.id);
                    }}
                  >
                    Review…
                  </Button>
                )}
              </li>
            ))}
          </ul>
        ) : radios.length === 0 ? (
          <p className={styles.muted}>No EdgeTX radio is saved. Plug one in and save it.</p>
        ) : (
          <>
            <fieldset className={styles.pick}>
              <legend>Radios</legend>
              <Checkbox label="All radios" checked={all} onChange={(e) => setPicked(e.target.checked ? radios.map((d) => d.id) : [])} />
              {radios.map((d) => (
                <Checkbox
                  key={d.id}
                  label={deviceName(d)}
                  detail={reachable(d.id) ? "Card connected" : "Not connected: applies when plugged in"}
                  checked={picked.includes(d.id)}
                  onChange={(e) => setPicked(e.target.checked ? [...picked, d.id] : picked.filter((x) => x !== d.id))}
                />
              ))}
            </fieldset>
            <Checkbox label="Keep each radio's overrides" checked={keep} onChange={(e) => setKeep(e.target.checked)} />
          </>
        )}
        {error && (
          <p role="alert" className={styles.error}>
            {error}
          </p>
        )}
      </div>
    </Dialog>
  );
}
