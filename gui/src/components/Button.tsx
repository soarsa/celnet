/**
 * Button — the one button primitive (primary / secondary / ghost / side-tinted).
 * Keyboard-first: a visible focus ring is inherited from the global :focus-visible
 * rule. Side-tinted variants drive click-to-trade (bid/offer).
 */

import styles from "./Button.module.css";

export interface ButtonProps
  extends React.ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: "primary" | "secondary" | "ghost" | "bid" | "offer";
  size?: "md" | "lg";
  kbd?: string;
}

export function Button({
  variant = "secondary",
  size = "md",
  kbd,
  children,
  className,
  ...rest
}: ButtonProps): React.ReactElement {
  return (
    <button
      className={[styles.btn, styles[variant], styles[size], className ?? ""]
        .filter(Boolean)
        .join(" ")}
      {...rest}
    >
      <span className={styles.label}>{children}</span>
      {kbd && <kbd className={styles.kbd}>{kbd}</kbd>}
    </button>
  );
}
