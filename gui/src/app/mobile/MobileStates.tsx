/**
 * Shared loading / empty / error states for the mobile status board — honest,
 * non-fabricated placeholders (a shimmer skeleton on first load, a plain empty note,
 * a soft error). Read-only; no retry buttons that would imply a mutation.
 */

import styles from "./MobileStatusApp.module.css";

/** A shimmer skeleton shown on FIRST load only (no data yet). */
export function MobileSkeleton({ rows = 4 }: { rows?: number }): React.ReactElement {
  return (
    <div className={styles.board} aria-busy="true" aria-label="loading">
      <div className={styles.summary}>
        {[0, 1, 2, 3].map((i) => (
          <div key={i} className={`${styles.statTile} ${styles.skel}`} />
        ))}
      </div>
      <ul className={styles.cardList}>
        {Array.from({ length: rows }, (_, i) => (
          <li key={i}>
            <div className={`${styles.card} ${styles.skel}`} style={{ height: 96 }} />
          </li>
        ))}
      </ul>
    </div>
  );
}

/** An honest empty state — "no deals yet", not a fake row. */
export function MobileEmpty({ message }: { message: string }): React.ReactElement {
  return (
    <div className={styles.state} role="status">
      <span className={styles.stateGlyph} aria-hidden>
        ◦
      </span>
      <p className={styles.stateText}>{message}</p>
    </div>
  );
}

/** A soft error state (the cache keeps the last good data when there was any). */
export function MobileError({ message }: { message: string }): React.ReactElement {
  return (
    <div className={styles.state} role="alert" data-tone="danger">
      <span className={styles.stateGlyph} aria-hidden>
        !
      </span>
      <p className={styles.stateText}>{message}</p>
    </div>
  );
}
