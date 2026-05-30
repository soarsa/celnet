/**
 * StatusRibbon — the slim bottom ribbon (GUI-DESIGN §2): connection/stream
 * health, sequence, LP-in-competition count, last-look horizon, the global 24h
 * clock, and a real measured P99 render time (principle 10 — the budget is an
 * instrument, not decor). The frame timer samples requestAnimationFrame deltas
 * and reports the rolling P99.
 */

import { useEffect, useRef, useState } from "react";
import { useApp } from "./AppContext";
import { useSecondClock } from "../hooks/useClock";
import { fmtClock } from "../lib/format";
import styles from "./StatusRibbon.module.css";

/** A rolling P99 of inter-frame render times, computed off a small ring. */
function useRenderP99(): number {
  const samples = useRef<number[]>([]);
  const last = useRef<number>(performance.now());
  const [p99, setP99] = useState(0);

  useEffect(() => {
    let raf = 0;
    let mounted = true;
    const loop = () => {
      if (!mounted) return;
      const now = performance.now();
      const dt = now - last.current;
      last.current = now;
      const ring = samples.current;
      ring.push(dt);
      if (ring.length > 120) ring.shift();
      raf = requestAnimationFrame(loop);
    };
    raf = requestAnimationFrame(loop);
    const id = setInterval(() => {
      const ring = [...samples.current].sort((a, b) => a - b);
      if (ring.length > 4) {
        const idx = Math.min(ring.length - 1, Math.floor(ring.length * 0.99));
        setP99(ring[idx] ?? 0);
      }
    }, 1000);
    return () => {
      mounted = false;
      cancelAnimationFrame(raf);
      clearInterval(id);
    };
  }, []);

  return p99;
}

export function StatusRibbon(): React.ReactElement {
  const app = useApp();
  const now = useSecondClock();
  const p99 = useRenderP99();

  const healthy = app.stream.rows.filter((r) => r.health === "HEALTHY").length;
  const resyncing = app.stream.rows.filter((r) => r.health === "RESYNCING").length;
  const totalGaps = app.stream.rows.reduce((acc, r) => acc + r.gaps, 0);
  const allHealthy = resyncing === 0;

  return (
    <footer className={styles.ribbon}>
      <span className={`${styles.item} ${allHealthy ? styles.ok : styles.warn}`}>
        <span className={styles.dot} aria-hidden>
          {allHealthy ? "◉" : "◐"}
        </span>
        {allHealthy ? "Stream healthy" : `Resyncing ${resyncing}`}
      </span>
      <span className={styles.sep} aria-hidden>
        ·
      </span>
      <span className={`num ${styles.item}`}>seq {app.stream.totalSeq.toString()}</span>
      <span className={styles.sep} aria-hidden>
        ·
      </span>
      <span className={`num ${styles.item}`}>
        {healthy}/{app.stream.rows.length} lines
      </span>
      <span className={styles.sep} aria-hidden>
        ·
      </span>
      <span className={`num ${styles.item}`}>{app.stream.lpCount} LPs in competition</span>
      <span className={styles.sep} aria-hidden>
        ·
      </span>
      <span className={`num ${styles.item} ${totalGaps === 0 ? styles.ok : styles.warn}`}>
        {totalGaps} gaps
      </span>

      <span className={styles.spacer} />

      <span className={styles.item} title="transport seam">
        {app.transport.label}
      </span>
      <span className={styles.sep} aria-hidden>
        ·
      </span>
      <span
        className={`num ${styles.item} ${p99 <= 16.6 ? styles.ok : styles.warn}`}
        title="P99 inter-frame render time (target ≤8.3ms @120Hz, ceiling 16.6ms)"
      >
        P99 render {p99.toFixed(1)}ms
      </span>
      <span className={styles.sep} aria-hidden>
        ·
      </span>
      <span className={`num ${styles.clock}`}>◷ {fmtClock(now)}</span>
    </footer>
  );
}
