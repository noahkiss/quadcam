import { ICONS, type IconName } from "./icons";
import styles from "./Icon.module.css";

export type { IconName };

export type IconTint = "blue" | "pink" | "green" | "yellow" | "red" | "mauve" | "sky" | "peach" | "teal" | "muted";

interface Props {
  name: IconName;
  size?: number;
  tint?: IconTint;
  className?: string;
}

/** A Solar icon. Decorative: the control around it carries the name. */
export function Icon({ name, size = 16, tint, className }: Props) {
  return (
    <i
      className={[styles.icon, tint && styles[tint], className].filter(Boolean).join(" ")}
      style={{ width: size, height: size }}
      aria-hidden="true"
      // The SVGs are constants bundled with the app.
      dangerouslySetInnerHTML={{ __html: ICONS[name] }}
    />
  );
}
