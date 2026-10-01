import { useToast } from "./toastStore";
import styles from "./Toast.module.css";

/** The live region for `toast()`. Mount it once. */
export function Toast() {
  const { message, error, seq } = useToast();
  return (
    <div role="status" aria-live="polite" className={[styles.toast, message && styles.show, error && styles.error].filter(Boolean).join(" ")}>
      <span key={seq}>{message}</span>
    </div>
  );
}
