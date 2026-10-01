import { forwardRef, useId, type InputHTMLAttributes, type ReactNode, type SelectHTMLAttributes } from "react";
import { Icon } from "./Icon";
import styles from "./Field.module.css";

interface FieldProps {
  label: ReactNode;
  hint?: ReactNode;
  /** Label beside the control instead of above it. */
  inline?: boolean;
  className?: string;
  children: (id: string, hintId: string | undefined) => ReactNode;
}

/** A label, one control and an optional hint, wired together by id. */
export function Field({ label, hint, inline, className, children }: FieldProps) {
  const id = useId();
  const hintId = hint ? `${id}-hint` : undefined;
  return (
    <div className={[styles.field, inline && styles.inline, className].filter(Boolean).join(" ")}>
      <label htmlFor={id} className={styles.label}>
        {label}
      </label>
      {children(id, hintId)}
      {hint && (
        <span id={hintId} className={styles.hint}>
          {hint}
        </span>
      )}
    </div>
  );
}

export type InputProps = InputHTMLAttributes<HTMLInputElement> & { mono?: boolean };

/** A text-like input: text, search, date, time, number. */
export const Input = forwardRef<HTMLInputElement, InputProps>(function Input({ className, mono, type = "text", ...rest }, ref) {
  return <input ref={ref} type={type} spellCheck={false} className={[styles.input, mono && styles.mono, className].filter(Boolean).join(" ")} {...rest} />;
});

export const Select = forwardRef<HTMLSelectElement, SelectHTMLAttributes<HTMLSelectElement>>(function Select({ className, ...rest }, ref) {
  return <select ref={ref} className={[styles.input, styles.select, className].filter(Boolean).join(" ")} {...rest} />;
});

interface TextFieldProps extends Omit<InputProps, "id"> {
  label: ReactNode;
  hint?: ReactNode;
  inline?: boolean;
}

export const TextField = forwardRef<HTMLInputElement, TextFieldProps>(function TextField({ label, hint, inline, ...rest }, ref) {
  return (
    <Field label={label} hint={hint} inline={inline}>
      {(id, hintId) => <Input ref={ref} id={id} aria-describedby={hintId} {...rest} />}
    </Field>
  );
});

interface SelectFieldProps extends Omit<SelectHTMLAttributes<HTMLSelectElement>, "id"> {
  label: ReactNode;
  hint?: ReactNode;
  inline?: boolean;
}

export const SelectField = forwardRef<HTMLSelectElement, SelectFieldProps>(function SelectField({ label, hint, inline, ...rest }, ref) {
  return (
    <Field label={label} hint={hint} inline={inline}>
      {(id, hintId) => <Select ref={ref} id={id} aria-describedby={hintId} {...rest} />}
    </Field>
  );
});

interface CheckboxProps extends Omit<InputHTMLAttributes<HTMLInputElement>, "type"> {
  label: ReactNode;
  /** A second line under the label. */
  detail?: ReactNode;
}

export const Checkbox = forwardRef<HTMLInputElement, CheckboxProps>(function Checkbox({ label, detail, className, ...rest }, ref) {
  return (
    <label className={[styles.check, className].filter(Boolean).join(" ")}>
      <input ref={ref} type="checkbox" {...rest} />
      <span className={styles.checkText}>
        <span>{label}</span>
        {detail && <span className={styles.detail}>{detail}</span>}
      </span>
    </label>
  );
});

interface SearchFieldProps extends Omit<InputProps, "type"> {
  label: string;
}

/** A search box with its icon; `label` is the accessible name. */
export const SearchField = forwardRef<HTMLInputElement, SearchFieldProps>(function SearchField({ label, className, ...rest }, ref) {
  return (
    <label className={[styles.search, className].filter(Boolean).join(" ")}>
      <Icon name="search" size={16} />
      <input ref={ref} type="search" aria-label={label} spellCheck={false} {...rest} />
    </label>
  );
});
