import { Button } from "./Button";
import { Icon } from "./Icon";
import type { Flag } from "../ipc/types";
import styles from "./FlagButton.module.css";

interface Props {
  flag: Flag;
  onFlag: (flag: Flag) => void;
  size?: "md" | "sm";
  /** Show the words next to the icons. */
  labels?: boolean;
}

/** Pick and Reject toggles. Pressing the one that is on clears the flag. */
export function FlagButtons({ flag, onFlag, size = "sm", labels }: Props) {
  return (
    <>
      <Button
        variant="ghost"
        size={size}
        icon="flag"
        aria-label={labels ? undefined : "Pick"}
        title="Pick (P)"
        aria-pressed={flag === "pick"}
        className={flag === "pick" ? styles.pick : undefined}
        onClick={() => onFlag(flag === "pick" ? "none" : "pick")}
      >
        {labels ? "Pick" : null}
      </Button>
      <Button
        variant="ghost"
        size={size}
        icon="close-circle"
        aria-label={labels ? undefined : "Reject"}
        title="Reject (X)"
        aria-pressed={flag === "reject"}
        className={flag === "reject" ? styles.reject : undefined}
        onClick={() => onFlag(flag === "reject" ? "none" : "reject")}
      >
        {labels ? "Reject" : null}
      </Button>
    </>
  );
}

/** The flag as a mark on a card or row. */
export function FlagMark({ flag }: { flag: Flag }) {
  if (flag === "none") return null;
  const pick = flag === "pick";
  return (
    <span title={pick ? "Pick" : "Rejected"} role="img" aria-label={pick ? "Pick" : "Rejected"} className={pick ? styles.markPick : styles.markReject}>
      <Icon name={pick ? "flag" : "close"} size={14} />
    </span>
  );
}
