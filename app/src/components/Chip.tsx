import type { ReactNode } from "react";
import { Icon, type IconName, type IconTint } from "./Icon";
import styles from "./Chip.module.css";

interface ChipProps {
  icon?: IconName;
  tint?: IconTint;
  /** `overlay` sits on a thumbnail. */
  kind?: "plain" | "overlay" | "accent";
  title?: string;
  children?: ReactNode;
}

/** A small label: a place, an aircraft, a count. */
export function Chip({ icon, tint, kind = "plain", title, children }: ChipProps) {
  return (
    <span className={[styles.chip, styles[kind]].join(" ")} title={title}>
      {icon && <Icon name={icon} size={13} tint={tint} />}
      {children}
    </span>
  );
}

/** The "agent" mark on a value an agent suggested. */
export function AgentBadge({ title }: { title?: string }) {
  return (
    <span className={styles.agent} title={title}>
      agent
    </span>
  );
}

/** A key name, as in a shortcut hint. */
export function Kbd({ children }: { children: ReactNode }) {
  return <kbd className={styles.kbd}>{children}</kbd>;
}
