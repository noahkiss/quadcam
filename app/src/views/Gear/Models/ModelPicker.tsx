import { Banner } from "../../../components/Banner";
import { Select } from "../../../components/Field";
import type { ModelDetail } from "../../../ipc/types";
import styles from "./Models.module.css";

interface Props {
  detail: ModelDetail;
  file: string | null;
  pick: (file: string) => void;
}

/** The model to edit, where it was read from, and the staged-edit note. */
export function ModelPicker({ detail, file, pick }: Props) {
  return (
    <>
      <div className={styles.bar}>
        <label className={styles.field}>
          Model
          <Select aria-label="Model" value={file ?? detail.view.file} onChange={(e) => pick(e.target.value)}>
            {detail.models.map((m) => (
              <option key={m.file} value={m.file}>
                {m.name || m.file}
                {m.selected ? " (selected on the radio)" : ""}
              </option>
            ))}
          </Select>
        </label>
        <span className={styles.source}>Read from the {detail.source}</span>
      </div>
      {detail.notes.map((n) => (
        <Banner key={n} kind="info" icon="info">
          {n}
        </Banner>
      ))}
    </>
  );
}
