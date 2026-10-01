import { forwardRef, useState, type InputHTMLAttributes } from "react";
import { Input } from "./Field";

interface Props extends Omit<InputHTMLAttributes<HTMLInputElement>, "value" | "onChange" | "defaultValue"> {
  value: string;
  /** Called with the new text when it differs, on Return or when the field loses focus. */
  onCommit: (value: string) => void;
  mono?: boolean;
}

/** A field that shows the core's value until the person types, and commits once. A value
 * that changes while the field has focus does not overwrite the typing. */
export const CommitInput = forwardRef<HTMLInputElement, Props>(function CommitInput({ value, onCommit, onKeyDown, onFocus, onBlur, ...rest }, ref) {
  const [draft, setDraft] = useState<string | null>(null);
  const commit = () => {
    if (draft != null && draft !== value) onCommit(draft);
    setDraft(null);
  };
  return (
    <Input
      ref={ref}
      {...rest}
      value={draft ?? value}
      onFocus={(e) => {
        setDraft(value);
        onFocus?.(e);
      }}
      onChange={(e) => {
        setDraft(e.target.value);
        // Date, time and select-like inputs commit at once.
        if (rest.type === "date" || rest.type === "time") {
          if (e.target.value !== value) onCommit(e.target.value);
        }
      }}
      onBlur={(e) => {
        commit();
        onBlur?.(e);
      }}
      onKeyDown={(e) => {
        if (e.key === "Enter") commit();
        onKeyDown?.(e);
      }}
    />
  );
});
