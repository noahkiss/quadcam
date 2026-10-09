// The Sim page's settings (sim design 9.3, the part S5 has): aircraft, view, picture and the two
// overlays. Each change saves at once through the settings file; there is no Save button.
import { Checkbox, Field, Select } from "../../../components/Field";
import { CommitInput } from "../../../components/CommitInput";
import { SegmentedControl } from "../../../components/SegmentedControl";
import type { SimPreset, SimUiSettings } from "../../../ipc/types";
import type { SimView } from "./Stage";
import styles from "./SimPage.module.css";

interface Props {
  presets: SimPreset[];
  profile: string;
  ui: SimView;
  /** The profile's own values, shown when a field is cleared. */
  profileCamera: { uptiltDeg: number; fovDeg: number; aspect: string } | null;
  onChange: (patch: Partial<SimUiSettings>) => void;
}

const num = (text: string, lo: number, hi: number): number | null => {
  const n = Number(text);
  return text.trim() !== "" && Number.isFinite(n) ? Math.min(hi, Math.max(lo, n)) : null;
};

export function SimSettings({ presets, profile, ui, profileCamera, onChange }: Props) {
  return (
    <>
      <Field label="Aircraft">
        {(id) => (
          <Select id={id} value={profile} onChange={(e) => onChange({ profile: e.target.value })}>
            {presets.map((p) => (
              <option key={p.id} value={p.id}>
                {p.label}
              </option>
            ))}
          </Select>
        )}
      </Field>
      <div className={styles.field}>
        <span className={styles.fieldLabel}>View</span>
        <SegmentedControl
          label="View"
          size="sm"
          value={ui.view}
          onChange={(view) => onChange({ view })}
          segments={[
            { value: "fpv", label: "FPV" },
            { value: "chase", label: "Chase" },
          ]}
        />
      </div>
      <Field label="Picture shape">
        {(id) => (
          <Select id={id} value={ui.aspect} onChange={(e) => onChange({ aspect: e.target.value as "16:9" | "4:3" })}>
            <option value="16:9">16:9</option>
            <option value="4:3">4:3</option>
          </Select>
        )}
      </Field>
      <div className={styles.two}>
        <Field label="Camera tilt (degrees)">
          {(id) => (
            <CommitInput id={id} type="number" min={0} max={60} value={String(ui.uptiltDeg)} onCommit={(t) => onChange({ uptilt_deg: num(t, 0, 60) })} placeholder={profileCamera ? String(profileCamera.uptiltDeg) : undefined} />
          )}
        </Field>
        <Field label="Field of view (degrees, diagonal)">
          {(id) => <CommitInput id={id} type="number" min={60} max={170} value={String(ui.fovDeg)} onCommit={(t) => onChange({ fov_deg: num(t, 60, 170) })} placeholder={profileCamera ? String(profileCamera.fovDeg) : undefined} />}
        </Field>
      </div>
      <Checkbox label="Stick display" checked={ui.stickDisplay} onChange={(e) => onChange({ stick_display: e.target.checked })} />
      <Checkbox label="OSD" checked={ui.osd} onChange={(e) => onChange({ osd: e.target.checked })} />
    </>
  );
}
