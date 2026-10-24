// A secret setting (an API key, a binding phrase). A stored one shows only the core's hint
// (first and last characters and the length) and a Replace… button; the value never comes
// back to the UI. Replace opens a password field; Keep current closes it and drops the draft.
import { useState, type ReactNode } from "react";
import { Button } from "./Button";
import { Field, Input } from "./Field";
import styles from "./Field.module.css";

interface SecretFieldProps {
  label: string;
  /** The core's hint of the stored value; none when nothing is stored. */
  hint?: string | null;
  value: string;
  onChange: (value: string) => void;
  /** Controls beside the field while a new value is typed (Save). */
  children?: ReactNode;
}

export function SecretField(p: SecretFieldProps) {
  // A new hint (a save) closes the field again.
  return <Secret key={p.hint ?? ""} {...p} />;
}

function Secret({ label, hint, value, onChange, children }: SecretFieldProps) {
  const [replacing, setReplacing] = useState(false);
  if (hint && !replacing) {
    return (
      <div className={styles.secret}>
        <Field label={label}>
          {(id) => (
            <output id={id} className={styles.secretHint}>
              {hint}
            </output>
          )}
        </Field>
        <Button size="sm" variant="secondary" onClick={() => setReplacing(true)}>
          Replace…
        </Button>
      </div>
    );
  }
  return (
    <div className={styles.secret}>
      <Field label={label}>{(id) => <Input id={id} type="password" autoComplete="off" autoFocus={replacing} value={value} onChange={(e) => onChange(e.target.value)} />}</Field>
      {children}
      {hint && (
        <Button
          size="sm"
          variant="ghost"
          onClick={() => {
            onChange("");
            setReplacing(false);
          }}
        >
          Keep Current
        </Button>
      )}
    </div>
  );
}
