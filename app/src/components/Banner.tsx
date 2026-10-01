import type { ReactNode } from "react";
import { Icon, type IconName, type IconTint } from "./Icon";
import styles from "./Banner.module.css";

interface Props {
  kind?: "info" | "warning" | "error";
  icon?: IconName;
  tint?: IconTint;
  children: ReactNode;
  action?: ReactNode;
}

/** A full-width notice above the content. */
export function Banner({ kind = "info", icon, tint, children, action }: Props) {
  return (
    <div className={[styles.banner, styles[kind]].join(" ")} role={kind === "error" ? "alert" : undefined}>
      {icon && <Icon name={icon} tint={tint} />}
      <span className={styles.text}>{children}</span>
      {action}
    </div>
  );
}
