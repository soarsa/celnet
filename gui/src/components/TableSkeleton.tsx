/**
 * TableSkeleton — a lightweight loading placeholder shown ONLY on a table's FIRST
 * load (a cache miss, before any data has ever arrived). On a tab switch back to an
 * already-loaded table the cached rows render instantly and this is never shown —
 * so a background stale-while-revalidate refresh does not flash a skeleton.
 *
 * Deliberately content-agnostic: a stack of shimmer bars sized to look like table
 * rows. Theme-aware via tokens (see the CSS module). `aria-hidden` + a polite
 * `role="status"` label keep it announced once without reading every bar.
 */
import styles from "./TableSkeleton.module.css";

export function TableSkeleton({
  rows = 6,
  label = "Loading…",
}: {
  /** How many shimmer rows to render. */
  rows?: number;
  /** The accessible status label announced while loading. */
  label?: string;
}): React.ReactElement {
  return (
    <div className={styles.wrap} role="status" aria-live="polite" aria-label={label}>
      {Array.from({ length: rows }, (_, i) => (
        <div key={i} className={styles.row} aria-hidden="true" />
      ))}
    </div>
  );
}
