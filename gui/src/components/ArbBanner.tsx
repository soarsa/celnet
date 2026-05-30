/**
 * ArbBanner — correctness is visible (GUI-DESIGN principle 8). Shows the
 * calibrated smile's arbitrage status from the contract's ArbReport: a calm
 * arb-free state (butterfly ≥ 0, calendar OK) or a named danger banner with the
 * failing constraint, never silently wrong.
 */

import type { ArbReport } from "../data/contract";
import styles from "./ArbBanner.module.css";

export function ArbBanner({ arb }: { arb: ArbReport }): React.ReactElement {
  const ok = arb.butterflyArbitrageFree && arb.calendarArbitrageFree;
  return (
    <div className={`${styles.banner} ${ok ? styles.ok : styles.danger}`} role="status">
      <span className={styles.glyph}>{ok ? "◉" : "⚠"}</span>
      <span className={styles.text}>
        {ok ? "arb-free" : "arbitrage"}
        <span className={styles.detail}>
          {arb.butterflyArbitrageFree ? "✓ butterfly ≥ 0" : "✕ butterfly"}
          {" · "}
          {arb.calendarArbitrageFree ? "✓ calendar" : "✕ calendar"}
        </span>
      </span>
      {!ok && <span className={styles.note}>{arb.note}</span>}
    </div>
  );
}
