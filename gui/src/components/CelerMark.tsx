/**
 * CelerMark — the Celer brand mark: a geometric PINWHEEL "C" of four coral
 * quadrant petals in a square. Pure inline SVG (no asset, no dependency). The
 * fill is `currentColor`, so the colour comes from CSS — default coral via the
 * lockup/rail wrappers (`color: var(--brand)`), but it inherits any text colour
 * if dropped elsewhere. The exact path is the authoritative Celer brand-kit mark
 * (viewBox 0 0 501 500); the wordmark is intentionally dropped — this is the
 * coral mark only.
 */

import styles from "./CelerMark.module.css";

/** The brand-kit pinwheel path (four quadrant petals). */
const PINWHEEL_PATH =
  "M500.12.7H262.61V238.21C393.78,238.21,500.12,131.88,500.12.7M262.49,262V499.55H500C500,368.38,393.67,262,262.49,262M238.7,499.55V262H1.19c0,131.17,106.34,237.51,237.51,237.51M1.19,238.19H238.7V.67C107.53.67,1.19,107,1.19,238.19";

export interface CelerMarkProps {
  /** Square edge length in px (default 24). */
  size?: number | undefined;
  className?: string | undefined;
  /** Accessible title; omit (default) to render the mark as decorative. */
  title?: string | undefined;
}

export function CelerMark({ size = 24, className, title }: CelerMarkProps): React.ReactElement {
  return (
    <svg
      className={[styles.mark, className ?? ""].filter(Boolean).join(" ")}
      width={size}
      height={size}
      viewBox="0 0 501 500"
      fill="currentColor"
      role={title ? "img" : undefined}
      aria-hidden={title ? undefined : true}
    >
      {title && <title>{title}</title>}
      <path d={PINWHEEL_PATH} />
    </svg>
  );
}

export interface CelerLockupProps {
  /** Mark edge length in px (default 22). The wordmark scales with it. */
  size?: number | undefined;
  /** Show the "a Celer Technologies product" caption (default true). */
  caption?: boolean | undefined;
  className?: string | undefined;
}

/**
 * CelerLockup — the co-brand lockup: the coral pinwheel mark + the "Celnet"
 * wordmark (Anaheim 600) + a small uppercase, letter-spaced "a Celer Technologies
 * product" caption. The mark instance lives in the left rail; this full lockup is
 * for splash/about/login surfaces — NOT the toolbar (the toolbar uses
 * [`CelnetWordmark`] to avoid showing the pinwheel twice).
 */
export function CelerLockup({
  size = 22,
  caption = true,
  className,
}: CelerLockupProps): React.ReactElement {
  return (
    <span className={[styles.lockup, className ?? ""].filter(Boolean).join(" ")}>
      <CelerMark size={size} className={styles.lockupMark} title="Celer Technologies" />
      <span className={styles.lockupText}>
        <span className={styles.wordmark}>Celnet</span>
        {caption && <span className={styles.caption}>a Celer Technologies product</span>}
      </span>
    </span>
  );
}

/**
 * CelnetWordmark — the product wordmark WITHOUT the pinwheel mark: "Celnet"
 * (Anaheim 600) over a small "Celer Technologies" parent tag. Used in the toolbar,
 * where the single brand mark already lives in the left rail — this leverages the
 * parent-company name without duplicating the logo.
 */
export function CelnetWordmark({
  className,
}: {
  className?: string | undefined;
}): React.ReactElement {
  return (
    <span className={[styles.lockupText, className ?? ""].filter(Boolean).join(" ")}>
      <span className={styles.wordmark}>Celnet</span>
      <span className={styles.caption}>Celer Technologies</span>
    </span>
  );
}
