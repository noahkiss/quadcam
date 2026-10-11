import type { ReactNode } from "react";
import { Icon, type IconName } from "../Icon";
import styles from "./EmptyState.module.css";

/** What an empty page or section is missing, in one line, with an optional action. It sits
 *  left-aligned where the content would be, never centred on the page. */
export function EmptyState({ icon, children, action }: { icon?: IconName; children: ReactNode; action?: ReactNode }) {
  return (
    <div className={styles.empty} data-component="empty-state">
      {icon && <Icon name={icon} size={20} tint="muted" />}
      <p className={styles.text}>{children}</p>
      {action && <div className={styles.action}>{action}</div>}
    </div>
  );
}
