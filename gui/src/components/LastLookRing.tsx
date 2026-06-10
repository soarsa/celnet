/**
 * LastLookRing — a thin SVG ring that depletes against a `valid_until` deadline
 * (GUI-DESIGN §3.4, §4.1). Turns `warn` under 1.5 s. The countdown is honest:
 * it reads the same nanosecond deadline the contract stamps on a Quote /
 * TradableToken, so the trader is never trading on an expired window.
 */

import { useCountdownClock } from "../hooks/useClock";
import { secondsUntil } from "../lib/format";
import styles from "./LastLookRing.module.css";

export interface LastLookRingProps {
  validUntilNanos: bigint;
  /** The full window length in seconds, for the depletion fraction. */
  windowSeconds: number;
  size?: number;
  label?: string;
}

export function LastLookRing({
  validUntilNanos,
  windowSeconds,
  size = 18,
  label = "last-look",
}: LastLookRingProps): React.ReactElement {
  const now = useCountdownClock(true);
  const remaining = secondsUntil(validUntilNanos, now);
  const frac = Math.max(0, Math.min(1, remaining / windowSeconds));
  const expiring = remaining <= 1.5;
  const expired = remaining <= 0;

  const r = (size - 3) / 2;
  const c = 2 * Math.PI * r;
  const dash = c * frac;

  return (
    // A live countdown is an ARIA `timer` (the APG role for a ticking numerical
    // counter); the explicit role also makes the accessible name valid here —
    // `aria-label` is prohibited on a generic <span>.
    <span role="timer" className={styles.wrap} aria-label={`${label} ${remaining.toFixed(1)} seconds`}>
      <svg width={size} height={size} className={styles.svg}>
        <circle
          cx={size / 2}
          cy={size / 2}
          r={r}
          className={styles.track}
          fill="none"
        />
        <circle
          cx={size / 2}
          cy={size / 2}
          r={r}
          fill="none"
          className={`${styles.arc} ${expiring ? styles.warn : ""} ${expired ? styles.expired : ""}`}
          strokeDasharray={`${dash} ${c}`}
          transform={`rotate(-90 ${size / 2} ${size / 2})`}
        />
      </svg>
      <span className={`num ${styles.secs} ${expiring ? styles.warnText : ""}`}>
        {expired ? "—" : `${remaining.toFixed(1)}s`}
      </span>
    </span>
  );
}
