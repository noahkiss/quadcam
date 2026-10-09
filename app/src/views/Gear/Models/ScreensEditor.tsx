import { Button } from "../../../components/Button";
import { CommitInput } from "../../../components/CommitInput";
import type { ModelDetail, ModelOp } from "../../../ipc/types";
import { nextScreen, parseSources } from "../../../lib/models";
import styles from "./Models.module.css";

interface Props {
  detail: ModelDetail;
  stage: (ops: ModelOp[]) => Promise<void>;
}

/** Telemetry screens: a value screen is up to 4 lines of up to 3 sources; a script screen names a script. */
export function ScreensEditor({ detail, stage }: Props) {
  const screens = detail.editors.screens;
  const next = nextScreen(screens.map((s) => s.index));
  const sensors = detail.view.sensors.map((s) => s.label);
  return (
    <section className={styles.section} aria-label="Telemetry screens">
      <h3>Telemetry screens</h3>
      <datalist id="screen-sources">
        {sensors.map((l) => (
          <option key={l} value={`{${l}}`} />
        ))}
        {["Tmr1", "Tmr2", "Tmr3"].map((t) => (
          <option key={t} value={t} />
        ))}
      </datalist>
      {screens.length === 0 && <p className={styles.muted}>This model has no telemetry screens.</p>}
      <ul className={styles.rows}>
        {screens.map((s) => {
          const n = s.index + 1;
          return (
            <li key={s.index}>
              <fieldset className={styles.row}>
                <legend className={styles.legend}>Screen {n}</legend>
                {s.kind === "VALUES" ? (
                  [0, 1, 2, 3].map((li) => (
                    <label key={li} className={`${styles.field} ${styles.wide}`}>
                      Line {li + 1}
                      <CommitInput
                        aria-label={`Screen ${n} line ${li + 1}`}
                        list="screen-sources"
                        value={(s.labels[li] ?? []).join(", ")}
                        onCommit={(v) => {
                          const lines = [0, 1, 2, 3].map((k) => (k === li ? parseSources(v) : (s.labels[k] ?? [])));
                          while (lines.length && lines[lines.length - 1].length === 0) lines.pop();
                          return stage([{ op: "set_screen_values", index: s.index, lines }]);
                        }}
                      />
                    </label>
                  ))
                ) : s.kind === "SCRIPT" ? (
                  <label className={styles.field}>
                    Script
                    <CommitInput aria-label={`Screen ${n} script`} maxLength={6} value={s.script ?? ""} onCommit={(v) => stage([{ op: "set_screen", index: s.index, script: v.trim() || null }])} />
                  </label>
                ) : (
                  <span className={styles.muted}>{s.kind.toLowerCase()} screen (edit in EdgeTX)</span>
                )}
                <Button size="sm" variant="danger-ghost" icon="trash-bin-trash" aria-label={`Remove screen ${n}`} onClick={() => stage([{ op: "set_screen", index: s.index, script: null }])} />
              </fieldset>
            </li>
          );
        })}
      </ul>
      {next != null && (
        <div className={styles.bar}>
          <Button size="sm" icon="add" onClick={() => stage([{ op: "set_screen_values", index: next, lines: [["Tmr1"]] }])}>
            Add value screen
          </Button>
        </div>
      )}
    </section>
  );
}
