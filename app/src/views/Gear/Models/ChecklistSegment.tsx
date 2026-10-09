// The radio's Checklists segment (design 6.3, WP9): the power-on checklist of one model, one
// item per line with an optional tick box (`=`), and whether the radio shows it. Staging the
// list sends the text through `gear_model_edit`; the apply sheet writes `MODELS/<name>.txt`.
import { useState } from "react";
import { Banner } from "../../../components/Banner";
import { Button } from "../../../components/Button";
import { Checkbox, Input } from "../../../components/Field";
import { checklistText, overWidth, parseChecklist, type CheckItem } from "../../../lib/models";
import type { DeviceRef } from "../slots";
import { ModelPicker } from "./ModelPicker";
import { StagedBar } from "./StagedBar";
import { useModel } from "./useModel";
import styles from "./Models.module.css";

export function ChecklistSegment({ d }: { d: DeviceRef }) {
  const device = d.device?.id ?? null;
  const m = useModel(device);
  const detail = m.detail;
  const shown = detail?.checklist ?? "";
  const [items, setItems] = useState<CheckItem[]>([]);
  // The list follows the core's text until the person edits it.
  const key = `${detail?.view.file}\n${shown}`;
  const [synced, setSynced] = useState<string | null>(null);
  if (synced !== key) {
    setSynced(key);
    setItems(parseChecklist(shown));
  }
  const width = detail?.checklist_width ?? null;
  const dirty = checklistText(items) !== checklistText(parseChecklist(shown));
  const over = overWidth(items, width);
  const edit = (i: number, patch: Partial<CheckItem>) => setItems(items.map((x, k) => (k === i ? { ...x, ...patch } : x)));
  const move = (i: number, by: number) => {
    const j = i + by;
    if (j < 0 || j >= items.length) return;
    const next = [...items];
    [next[i], next[j]] = [next[j], next[i]];
    setItems(next);
  };
  return (
    <div className={styles.segment}>
      {!device && <p className={styles.muted}>Save this radio from Overview to edit its checklists.</p>}
      <StagedBar device={device} />
      {m.error && (
        <Banner kind="error" icon="danger-triangle">
          {m.error}
        </Banner>
      )}
      {detail && (
        <>
          <ModelPicker detail={detail} file={m.file} pick={m.pick} />
          <section className={styles.section} aria-label="Checklist">
            <h3>Checklist</h3>
            <Checkbox label="Show the checklist when the model loads" checked={detail.view.checklist} onChange={(e) => m.stage([{ op: "set_checklist", enabled: e.target.checked }])} />
            <ul className={styles.items} aria-label="Checklist items">
              {items.map((it, i) => {
                const n = (it.tick ? 1 : 0) + it.text.length;
                return (
                  // Items have no id; the list is edited in place, so the position is the key.
                  <li key={i} className={styles.item}>
                    <Checkbox label="Tick box" checked={it.tick} onChange={(e) => edit(i, { tick: e.target.checked })} />
                    <Input aria-label={`Item ${i + 1}`} value={it.text} onChange={(e) => edit(i, { text: e.target.value })} />
                    <span className={styles.count} data-over={width != null && n > width} aria-label={`Item ${i + 1} length`}>
                      {width ? `${n}/${width}` : n}
                    </span>
                    <Button size="sm" icon="arrow-up" aria-label={`Move item ${i + 1} up`} disabled={i === 0} onClick={() => move(i, -1)} />
                    <Button size="sm" icon="arrow-down" aria-label={`Move item ${i + 1} down`} disabled={i === items.length - 1} onClick={() => move(i, 1)} />
                    <Button size="sm" variant="danger-ghost" icon="trash-bin-trash" aria-label={`Remove item ${i + 1}`} onClick={() => setItems(items.filter((_, k) => k !== i))} />
                  </li>
                );
              })}
            </ul>
            {items.length === 0 && <p className={styles.muted}>This model has no checklist items.</p>}
            {over > 0 && (
              <p className={styles.error} role="alert">
                {over === 1 ? "One item is" : `${over} items are`} over {width} characters, the width of the radio's screen.
              </p>
            )}
            <div className={styles.bar}>
              <Button size="sm" icon="add" onClick={() => setItems([...items, { tick: true, text: "" }])}>
                Add item
              </Button>
              <Button variant="primary" size="sm" disabled={!dirty || over > 0} onClick={() => m.stage([], checklistText(items))}>
                Stage checklist
              </Button>
            </div>
          </section>
        </>
      )}
    </div>
  );
}
