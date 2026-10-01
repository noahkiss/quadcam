import { forwardRef, type ButtonHTMLAttributes, type ReactNode } from "react";
import { Icon, type IconName } from "./Icon";
import styles from "./Button.module.css";

export type ButtonVariant = "primary" | "secondary" | "ghost" | "danger" | "danger-ghost";

export interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: ButtonVariant;
  size?: "md" | "sm";
  icon?: IconName;
  /** Icon after the label. */
  iconEnd?: IconName;
  children?: ReactNode;
}

/** A push button. Without children it is icon-only and needs `aria-label`. */
export const Button = forwardRef<HTMLButtonElement, ButtonProps>(function Button(
  { variant = "secondary", size = "md", icon, iconEnd, children, className, type = "button", ...rest },
  ref,
) {
  const iconOnly = children == null || children === "";
  return (
    <button
      ref={ref}
      type={type}
      className={[styles.button, styles[variant], styles[size], iconOnly && styles.iconOnly, className].filter(Boolean).join(" ")}
      {...rest}
    >
      {icon && <Icon name={icon} size={size === "sm" ? 14 : 16} />}
      {children}
      {iconEnd && <Icon name={iconEnd} size={size === "sm" ? 14 : 16} />}
    </button>
  );
});
