import { useId, type ReactNode } from "react";
import { Icon, type IconName } from "../Icon";
import styles from "./PageHeader.module.css";

export interface PageHeaderProps {
  title: ReactNode;
  /** One muted line under the title: kind · state · one fact. Never a sentence. */
  subtitle?: ReactNode;
  /** A large icon in a tile before the title (device pages). */
  icon?: IconName;
  /** Buttons on the right, primary last. A destructive one goes in a `MoreMenu`. */
  actions?: ReactNode;
  /** The heading's id, for the page's `aria-labelledby`. */
  id?: string;
}

/** The top of every page: the title with an optional subtitle on the left and the actions
 *  on the right, on one grid. */
export function PageHeader({ title, subtitle, icon, actions, id }: PageHeaderProps) {
  const own = useId();
  return (
    <header className={[styles.header, icon && styles.withIcon].filter(Boolean).join(" ")} data-component="page-header">
      {icon && (
        <span className={styles.icon}>
          <Icon name={icon} size={28} />
        </span>
      )}
      <div className={styles.text}>
        <h2 id={id ?? own} className={styles.title}>
          {title}
        </h2>
        {subtitle != null && subtitle !== "" && <span className={styles.sub}>{subtitle}</span>}
      </div>
      {actions && <div className={styles.actions}>{actions}</div>}
    </header>
  );
}
