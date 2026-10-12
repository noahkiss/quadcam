// The radio's Settings segment: its own settings (`RADIO/radio.yml`), grouped as the radio's
// Radio Setup screen groups them. Each change stages through `gear_radio_settings_edit` into
// the radio's one "Radio settings" change; nothing reaches the card until the apply sheet
// writes it.
import { useId, useState } from "react";
import { Banner } from "../../../components/Banner";
import { Button } from "../../../components/Button";
import { CommitInput } from "../../../components/CommitInput";
import { Select } from "../../../components/Field";
import { Badge } from "../../../components/dash/Badge";
import { EmptyState } from "../../../components/dash/EmptyState";
import { SectionCard } from "../../../components/dash/SectionCard";
import type { RadioOp, RadioSetting, RadioSettingsDetail } from "../../../ipc/types";
import { sourceText } from "../../../lib/models";
import { numberRange, numberRaw, numberShown, numberStep, openRadioChange, radioOps, shownValue, stagedKeys, textError, toggleOn, toggleRaw } from "../../../lib/radioSettings";
import { useStore } from "../../../store";
import type { DeviceRef } from "../slots";
import { useRadioSettings } from "./useRadioSettings";
import styles from "./RadioSettings.module.css";

export function RadioSettingsSegment({ d }: { d: DeviceRef }) {
  const device = d.device?.id ?? null;
  const r = useRadioSettings(device);
  const change = useStore((s) => openRadioChange(s.changes, device));
  const staged = stagedKeys(change);
  return (
    <div className={styles.segment}>
      {!device && <EmptyState icon="info">Save this radio from Overview to edit its settings.</EmptyState>}
      <StagedBar device={device} />
      {r.error && (
        <Banner kind="error" icon="danger-triangle">
          {r.error}
        </Banner>
      )}
      {r.note && <p className={styles.note}>{r.note}</p>}
      {r.detail && (
        <>
          <p className={styles.source}>{[r.detail.board, r.detail.version && `EdgeTX ${r.detail.version}`, sourceText(r.detail.source)].filter(Boolean).join(" · ")}</p>
          {r.detail.view.sections.map((section) => (
            <SectionCard key={section} title={section}>
              <div className={styles.form}>
                {r.detail!.view.settings
                  .filter((s) => s.section === section)
                  .map((s) => (
                    <SettingRow key={s.key} setting={s} base={r.base?.view.settings.find((b) => b.key === s.key)?.value ?? null} staged={staged.has(s.key)} stage={r.stage} />
                  ))}
              </div>
            </SectionCard>
          ))}
          <ModelSection detail={r.detail} base={r.base} staged={staged.has("currModel")} stage={r.stage} />
        </>
      )}
    </div>
  );
}

/** The radio's staged settings, with Review and Undo. */
function StagedBar({ device }: { device: string | null }) {
  const change = useStore((s) => openRadioChange(s.changes, device));
  const open = useStore((s) => s.openApply);
  const discard = useStore((s) => s.discardChange);
  if (!device || !change) return null;
  const n = radioOps(change).length;
  return (
    <Banner
      kind="info"
      icon="info"
      tint="blue"
      action={
        <>
          <Button size="sm" onClick={() => open(device, change.id)}>
            Review…
          </Button>
          <Button size="sm" variant="danger-ghost" onClick={() => discard(change.id)}>
            Undo settings edits
          </Button>
        </>
      }
    >
      {n === 1 ? "1 setting is staged." : `${n} settings are staged.`} The radio changes when you apply them.
    </Banner>
  );
}

interface RowProps {
  setting: RadioSetting;
  /** The value on the radio, without staged edits. */
  base: string | null;
  staged: boolean;
  stage: (ops: RadioOp[]) => Promise<void>;
}

function SettingRow({ setting: s, base, staged, stage }: RowProps) {
  const id = useId();
  const hintId = useId();
  const [error, setError] = useState<string | null>(null);
  const set = (value: string) => {
    setError(null);
    void stage([{ op: "set_scalar", key: s.key, value }]);
  };
  const k = s.kind;
  const value = s.value ?? null;
  const changed = staged && value !== base;
  const hint = error ?? (changed ? `On the radio: ${shownValue(s, base)}` : k.type === "number" ? rangeText(k) : null);
  if (value == null) {
    return (
      <>
        <span className={styles.label}>{s.label}</span>
        <span className={styles.muted}>Not on this card</span>
      </>
    );
  }
  let control;
  switch (k.type) {
    case "choice":
      control = (
        <Select id={id} aria-describedby={hint ? hintId : undefined} value={value} onChange={(e) => set(e.target.value)}>
          {k.choices.map((c) => (
            <option key={c.value} value={c.value}>
              {c.label}
            </option>
          ))}
        </Select>
      );
      break;
    case "number":
      control = (
        <span className={styles.number}>
          <CommitInput
            id={id}
            type="number"
            inputMode="decimal"
            aria-describedby={hint ? hintId : undefined}
            aria-invalid={error ? true : undefined}
            min={numberRange(k)[0]}
            max={numberRange(k)[1]}
            step={numberStep(k)}
            value={numberShown(k, value)}
            onCommit={(v) => {
              const r = numberRaw(k, v);
              if ("error" in r) setError(r.error);
              else set(r.raw);
            }}
          />
          {k.unit && <span className={styles.unit}>{k.unit}</span>}
        </span>
      );
      break;
    case "toggle":
      control = <input id={id} className={styles.check} type="checkbox" aria-describedby={hint ? hintId : undefined} checked={toggleOn(k, value)} onChange={(e) => set(toggleRaw(k, e.target.checked))} />;
      break;
    case "text":
      control = (
        <CommitInput
          id={id}
          aria-describedby={hint ? hintId : undefined}
          aria-invalid={error ? true : undefined}
          maxLength={k.max_len}
          value={value}
          onCommit={(v) => {
            const e = textError(k.max_len, v);
            if (e) setError(e);
            else set(v);
          }}
        />
      );
      break;
  }
  return (
    <>
      <label className={styles.label} htmlFor={id}>
        {s.label}
      </label>
      <div className={styles.control}>
        {control}
        {changed && <Badge tone="accent">Staged</Badge>}
        {hint && (
          <span id={hintId} className={error ? styles.error : styles.hint} role={error ? "alert" : undefined}>
            {hint}
          </span>
        )}
      </div>
    </>
  );
}

const rangeText = (k: Extract<RadioSetting["kind"], { type: "number" }>) => {
  const [lo, hi] = numberRange(k);
  // The unit shows beside the field.
  return `${lo} to ${hi}`;
};

/** The model the radio selects when it starts. */
function ModelSection({ detail, base, staged, stage }: { detail: RadioSettingsDetail; base: RadioSettingsDetail | null; staged: boolean; stage: (ops: RadioOp[]) => Promise<void> }) {
  const id = useId();
  const hintId = useId();
  const selected = detail.view.selected_model ?? "";
  const was = base?.view.selected_model ?? null;
  const changed = staged && selected !== was;
  const name = (f: string | null) => detail.models.find((m) => m.file === f)?.name || f || "None";
  if (!detail.models.length) return null;
  return (
    <SectionCard title="Model">
      <div className={styles.form}>
        <label className={styles.label} htmlFor={id}>
          Selected model
        </label>
        <div className={styles.control}>
          <Select id={id} aria-describedby={changed ? hintId : undefined} value={selected} onChange={(e) => void stage([{ op: "select_model", file: e.target.value }])}>
            {!selected && <option value="">None</option>}
            {detail.models.map((m) => (
              <option key={m.file} value={m.file}>
                {m.name || m.file}
              </option>
            ))}
          </Select>
          {changed && <Badge tone="accent">Staged</Badge>}
          {changed && (
            <span id={hintId} className={styles.hint}>
              On the radio: {name(was)}
            </span>
          )}
        </div>
      </div>
    </SectionCard>
  );
}
