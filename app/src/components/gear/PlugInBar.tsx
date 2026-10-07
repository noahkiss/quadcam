import { Button } from "../Button";
import { Icon } from "../Icon";
import styles from "./gear.module.css";

interface Props {
  /** Staged changes for this device that are ready. */
  count: number;
  onReview: () => void;
}

/** The bar at the top of a device page when it has staged changes: nothing is applied on
 *  its own; Review opens the apply sheet. */
export function PlugInBar({ count, onReview }: Props) {
  if (count <= 0) return null;
  return (
    <div className={styles.plugBar} role="status" data-component="plug-in-bar">
      <Icon name="sliders" tint="blue" />
      <span>{count === 1 ? "1 change ready" : `${count} changes ready`}</span>
      <Button size="sm" variant="primary" onClick={onReview}>
        Review…
      </Button>
    </div>
  );
}
