import { useId, type ReactNode } from "react";
import { Button, type ButtonVariant } from "../Button";
import { Icon, type IconName } from "../Icon";
import type { MenuItem } from "../Menu";
import { MoreMenu } from "./MoreMenu";
import styles from "./PageHeader.module.css";

/** A header button that folds into the `…` menu when the header is narrow. */
export interface HeaderAction {
  label: string;
  run: () => void;
  icon?: IconName;
  variant?: ButtonVariant;
  disabled?: boolean;
  /** The button's accessible name, when it differs from its label. */
  ariaLabel?: string;
}

export interface PageHeaderProps {
  title: ReactNode;
  /** One muted line under the title: kind · state · one fact, or a `SummaryLine`. Never a
   *  sentence. */
  subtitle?: ReactNode;
  /** A large icon in a tile before the title (device pages). */
  icon?: IconName;
  /** Controls on the right that always show (a pop-up, the main action). */
  actions?: ReactNode;
  /** Buttons on the right, before `actions`, that fold into the `…` menu in a narrow header. */
  fold?: (HeaderAction | null | false)[];
  /** Rare and destructive actions: always in the `…` menu. */
  more?: (MenuItem | null)[];
  /** The `…` menu's accessible name. */
  moreLabel?: string;
  /** The heading's id, for the page's `aria-labelledby`. */
  id?: string;
}

/** The top of every page: the title with an optional subtitle on the left and the actions
 *  on the right, on one grid. In a narrow header the actions wrap under the title and the
 *  `fold` buttons join the `…` menu. */
export function PageHeader({ title, subtitle, icon, actions, fold, more, moreLabel = "More actions", id }: PageHeaderProps) {
  const own = useId();
  const folded = (fold ?? []).filter((a): a is HeaderAction => !!a);
  const asItems: MenuItem[] = folded.map((a) => ({ label: a.label, icon: a.icon, disabled: a.disabled, run: a.run }));
  const narrowItems = [...asItems, ...(asItems.length && more?.length ? [null] : []), ...(more ?? [])];
  const hasActions = !!actions || folded.length > 0 || !!more?.length;
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
      {hasActions && (
        <div className={[styles.actions, actions && styles.controls].filter(Boolean).join(" ")}>
          {folded.map((a) => (
            <Button key={a.label} className={styles.wide} size="sm" variant={a.variant} icon={a.icon} disabled={a.disabled} aria-label={a.ariaLabel} onClick={a.run}>
              {a.label}
            </Button>
          ))}
          {!!more?.length && (
            <span className={styles.wide}>
              <MoreMenu label={moreLabel} items={more} />
            </span>
          )}
          {narrowItems.length > 0 && (
            <span className={styles.narrow}>
              <MoreMenu label={moreLabel} items={narrowItems} />
            </span>
          )}
          {actions}
        </div>
      )}
    </header>
  );
}
