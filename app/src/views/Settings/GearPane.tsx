import { useStore } from "../../store";
import { Checkbox, SelectField, TextField } from "../../components/Field";
import type { DeviceKind } from "../../ipc/types";
import { KIND_LABEL } from "../../lib/gear";
import { tilde } from "../../lib/format";
import { AUTOMATIONS, GEAR_KINDS, type Draft, type GearDraft, type ShownAutomation } from "./draft";
import styles from "./SettingsSheet.module.css";

const AUTOMATION_LABEL: Record<ShownAutomation, string> = { backup: "Back up", apply_ready: "Apply ready changes", blackbox: "Pull blackbox" };

/** The steps that make sense for a kind: Apply only for a device QuadCam writes settings to. */
const OFFERED: Record<ShownAutomation, DeviceKind[]> = {
  backup: GEAR_KINDS,
  apply_ready: ["radio", "fc", "elrs_tx", "elrs_rx"],
  blackbox: ["fc"],
};

export function GearPane({ d, setD }: { d: Draft; setD: (f: (d: Draft) => Draft) => void }) {
  const home = useStore((s) => s.home);
  const gearDir = useStore((s) => s.gear?.gear_dir);
  const g = d.gear;
  const set = (patch: Partial<GearDraft>) => setD((x) => ({ ...x, gear: { ...x.gear, ...patch } }));
  const cue = (patch: Partial<GearDraft["cues"]>) => set({ cues: { ...g.cues, ...patch } });
  const toggle = (k: DeviceKind, a: ShownAutomation, on: boolean) => {
    const cur = g.onConnect[k] || [];
    set({ onConnect: { ...g.onConnect, [k]: AUTOMATIONS.filter((x) => (x === a ? on : cur.includes(x))) } });
  };
  return (
    <section>
      <h3>Preview</h3>
      <Checkbox label="Show the Sim page" checked={g.simPreview} onChange={(e) => set({ simPreview: e.target.checked })} />
      <Checkbox label="Betaflight flashing (preview)" checked={g.bfFlashPreview} onChange={(e) => set({ bfFlashPreview: e.target.checked })} />
      <Checkbox label="Show the ELRS tools" checked={g.elrsPreview} onChange={(e) => set({ elrsPreview: e.target.checked })} />
      <h3>Backups</h3>
      {gearDir && <p className={`${styles.sub} selectable`}>{tilde(gearDir, home)}</p>}
      <Checkbox label="Back up on connect" checked={g.autoBackup} onChange={(e) => set({ autoBackup: e.target.checked })} />
      <div className={styles.row}>
        <TextField label="Keep recent backups" type="number" min={1} max={1000} step={1} value={g.keepRecent} onChange={(e) => set({ keepRecent: e.target.value })} />
        <TextField label="Then one a week for (weeks)" type="number" min={0} max={520} step={1} value={g.keepWeeks} onChange={(e) => set({ keepWeeks: e.target.value })} />
      </div>
      <Checkbox label="Then keep one a month" checked={g.keepMonthly} onChange={(e) => set({ keepMonthly: e.target.checked })} />
      <div className={styles.row}>
        <TextField label="USB power warning (min, 0 for off)" type="number" min={0} max={240} step={1} value={g.usbMinutes} onChange={(e) => set({ usbMinutes: e.target.value })} />
      </div>

      <h3>Blackbox</h3>
      <Checkbox label="Erase blackbox after download" checked={g.eraseBlackbox} onChange={(e) => set({ eraseBlackbox: e.target.checked })} />
      <Checkbox label="Read through USB disk mode first (not proven)" checked={g.blackboxMsc} onChange={(e) => set({ blackboxMsc: e.target.checked })} />

      <h3>Firmware</h3>
      <Checkbox label="Check for firmware updates daily" checked={g.firmwareDaily} onChange={(e) => set({ firmwareDaily: e.target.checked })} />

      <h3>When a device is plugged in</h3>
      <table className={styles.modules} aria-label="Steps on connect">
        <thead>
          <tr>
            <th scope="col">Device</th>
            {AUTOMATIONS.map((a) => (
              <th key={a} scope="col">
                {AUTOMATION_LABEL[a]}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {GEAR_KINDS.map((k) => (
            <tr key={k}>
              <th scope="row">{KIND_LABEL[k]}</th>
              {AUTOMATIONS.map((a) => (
                <td key={a}>
                  {OFFERED[a].includes(k) && (
                    <input
                      type="checkbox"
                      aria-label={`${AUTOMATION_LABEL[a]}: ${KIND_LABEL[k]}`}
                      checked={(g.onConnect[k] || []).includes(a)}
                      disabled={a === "backup" && !g.autoBackup}
                      onChange={(e) => toggle(k, a, e.target.checked)}
                    />
                  )}
                </td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>

      <h3>Cues</h3>
      <Checkbox label="Mute all cues" checked={g.cues.mute} onChange={(e) => cue({ mute: e.target.checked })} />
      <fieldset className={styles.fieldset} disabled={g.cues.mute}>
        <legend className={styles.label}>Play a cue for</legend>
        <Checkbox label="Done, safe to unplug" checked={g.cues.safe_to_unplug} onChange={(e) => cue({ safe_to_unplug: e.target.checked })} />
        <Checkbox label="Still inserted" checked={g.cues.still_inserted} onChange={(e) => cue({ still_inserted: e.target.checked })} />
        <Checkbox label="Step failed" checked={g.cues.step_failed} onChange={(e) => cue({ step_failed: e.target.checked })} />
        <Checkbox label="Unplug now" checked={g.cues.unplug_now} onChange={(e) => cue({ unplug_now: e.target.checked })} />
      </fieldset>
      <fieldset className={styles.fieldset} disabled={g.cues.mute}>
        <legend className={styles.label}>Play cues as</legend>
        <Checkbox label="Speech" checked={g.cues.speech} onChange={(e) => cue({ speech: e.target.checked })} />
        <Checkbox label="Sound" checked={g.cues.sound} onChange={(e) => cue({ sound: e.target.checked })} />
        <Checkbox label="Notification" checked={g.cues.notification} onChange={(e) => cue({ notification: e.target.checked })} />
      </fieldset>
      <div className={styles.row}>
        <SelectField label="Voice" value={g.cues.voice_source} onChange={(e) => cue({ voice_source: e.target.value as GearDraft["cues"]["voice_source"] })}>
          <option value="macos">macOS</option>
          <option value="voice_pack" disabled>
            Voice pack
          </option>
        </SelectField>
        <TextField label="macOS voice" placeholder="System voice" value={g.cues.voice} onChange={(e) => cue({ voice: e.target.value })} />
      </div>
      <Checkbox label="Quiet hours for speech and sound" checked={g.quiet} onChange={(e) => set({ quiet: e.target.checked })} />
      {g.quiet && (
        <div className={styles.row}>
          <TextField label="From" type="time" value={g.quietStart} onChange={(e) => set({ quietStart: e.target.value })} />
          <TextField label="To" type="time" value={g.quietEnd} onChange={(e) => set({ quietEnd: e.target.value })} />
        </div>
      )}
    </section>
  );
}
