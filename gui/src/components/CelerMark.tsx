/**
 * CelnetMark — the Celnet brand mark: a sleek geometric "C" arc.
 * Pure inline SVG (no asset, no dependency). The fill is `currentColor`, so the
 * colour comes from CSS — default coral via the lockup/rail wrappers (`color: var(--brand)`),
 * but it inherits any text colour if dropped elsewhere.
 */

import styles from "./CelerMark.module.css";

/** The Celnet geometric mark path (high-precision C arc). */
const CELNET_MARK_PATH =
  "M420 120 A 210 210 0 1 0 420 380 L 350 330 A 130 130 0 1 1 350 170 Z";

export interface CelnetMarkProps {
  /** Square edge length in px (default 24). */
  size?: number | undefined;
  className?: string | undefined;
  /** Accessible title; omit (default) to render the mark as decorative. */
  title?: string | undefined;
}

export function CelnetMark({ size = 24, className, title }: CelnetMarkProps): React.ReactElement {
  return (
    <svg
      className={[styles.mark, className ?? ""].filter(Boolean).join(" ")}
      width={size}
      height={size}
      viewBox="0 0 500 500"
      fill="currentColor"
      role={title ? "img" : undefined}
      aria-hidden={title ? undefined : true}
    >
      {title && <title>{title}</title>}
      <path d={CELNET_MARK_PATH} />
    </svg>
  );
}

/** Backward-compatible alias for existing imports. */
export const CelerMark = CelnetMark;
export type CelerMarkProps = CelnetMarkProps;

export interface CelnetLockupProps {
  /** Mark edge length in px (default 22). The wordmark scales with it. */
  size?: number | undefined;
  /** Show optional caption (default false). */
  caption?: boolean | undefined;
  className?: string | undefined;
}

export function CelnetLockup({
  size = 22,
  caption = false,
  className,
}: CelnetLockupProps): React.ReactElement {
  return (
    <span className={[styles.lockup, className ?? ""].filter(Boolean).join(" ")}>
      <CelnetMark size={size} className={styles.lockupMark} title="Celnet" />
      <span className={styles.lockupText}>
        <span className={styles.wordmark}>Celnet</span>
        {caption && <span className={styles.caption}>Derivatives Platform</span>}
      </span>
    </span>
  );
}

/** Backward-compatible alias for existing imports. */
export const CelerLockup = CelnetLockup;
export type CelerLockupProps = CelnetLockupProps;

/**
 * CelnetWordmark — the product wordmark without the mark icon: "Celnet".
 */
export function CelnetWordmark({
  className,
}: {
  className?: string | undefined;
}): React.ReactElement {
  return (
    <span className={[styles.lockupText, className ?? ""].filter(Boolean).join(" ")}>
      <span className={styles.wordmark}>Celnet</span>
    </span>
  );
}

