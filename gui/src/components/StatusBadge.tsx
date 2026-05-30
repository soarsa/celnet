/**
 * StatusBadge — per-row stream health (GUI-DESIGN §4.2): ◉ healthy / ◐ resyncing
 * / ○ stale, mapping to the contract's seq/resync state. Correctness is visible
 * (principle 8): the badge carries glyph + color + a screen-reader label, never
 * color alone (§7).
 */

import type { StreamHealth } from "../data/contract";
import styles from "./StatusBadge.module.css";

const GLYPH: Record<StreamHealth, string> = {
  HEALTHY: "◉",
  RESYNCING: "◐",
  STALE: "○",
};

const LABEL: Record<StreamHealth, string> = {
  HEALTHY: "stream healthy",
  RESYNCING: "resyncing",
  STALE: "stale",
};

export function StatusBadge({ health }: { health: StreamHealth }): React.ReactElement {
  return (
    <span
      className={`${styles.badge} ${styles[health.toLowerCase()]}`}
      role="img"
      aria-label={LABEL[health]}
      title={LABEL[health]}
    >
      {GLYPH[health]}
    </span>
  );
}
