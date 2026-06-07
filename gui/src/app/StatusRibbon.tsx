/**
 * StatusRibbon — the slim bottom ribbon (GUI-DESIGN §2): connection/stream
 * health, sequence, LP-in-competition count, last-look horizon, the global 24h
 * clock, and a real measured P99 render time (principle 10 — the budget is an
 * instrument, not decor). The frame timer samples requestAnimationFrame deltas
 * and reports the rolling P99.
 *
 * It also surfaces the SERVER's own observability — distilled from the live
 * heartbeats (`StreamApi.observability`): the drain-side price-compute p99, the
 * exact ring conflation-drop count, and the surface-version / correlation
 * provenance echo. These are honest server measurements (the ribbon shows "—"
 * until the first beat lands — never a fabricated zero), complementing the
 * client-side render p99 above.
 */

import { useEffect, useRef, useState } from "react";
import { useApp } from "./AppContext";
import { useSecondClock } from "../hooks/useClock";
import type { ServerObservability } from "../hooks/useStreamSession";
import { fmtClock, fmtLatencyNanos } from "../lib/format";
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

/** Compact UTC stamp "YYYY-MM-DD HH:MM:SSZ" from the build-time ISO string. */
function fmtBuildTime(iso: string): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  const p = (n: number) => n.toString().padStart(2, "0");
  return `${d.getUTCFullYear()}-${p(d.getUTCMonth() + 1)}-${p(d.getUTCDate())} ${p(d.getUTCHours())}:${p(d.getUTCMinutes())}:${p(d.getUTCSeconds())}Z`;
}

/**
 * The server-observability cluster of the ribbon — drain-side price p99, the
 * exact ring conflation-drop count, and the surface/correlation provenance echo.
 * A pure component over [`ServerObservability`] (no hooks), so it renders the
 * SAME honest output for the live WS heartbeat and the offline mock heartbeat,
 * and is unit-testable with hand-pinned values. Renders "—" until the first beat
 * lands (`received === false`) — never a fabricated zero presented as a measurement.
 */
export function ServerObservabilityItems({
  observability: obs,
}: {
  observability: ServerObservability;
}): React.ReactElement {
  const dropsClean = obs.conflationDrops === 0n;
  const surfaceVersion = obs.surfaceVersion;
  const correlationId = obs.correlationId;
  return (
    <>
      <span className={styles.sep} aria-hidden>
        ·
      </span>
      {/* Server price-compute p99 (drain-side HdrHistogram), distinct from the
          client render p99 — the trader sees both ends of the latency budget. */}
      <span
        className={`num ${styles.item}`}
        data-testid="server-p99"
        title="Server price-compute P99 (drain-side HdrHistogram, off the pinned hot core) — reported on the stream heartbeat"
      >
        server P99 {obs.received ? fmtLatencyNanos(obs.serverPriceP99Nanos) : "—"}
      </span>
      <span className={styles.sep} aria-hidden>
        ·
      </span>
      {/* Exact ring conflation-drop count (celnet-fanout `received + skipped ==
          produced`) — 0 ⇒ the consumer never lagged. */}
      <span
        className={`num ${styles.item} ${obs.received && !dropsClean ? styles.warn : styles.ok}`}
        data-testid="conflation-drops"
        title="Ticks the server's fan-out ring conflated (dropped under back-pressure) since open — exact skip count from the stream heartbeat"
      >
        {obs.received ? `${obs.conflationDrops.toString()} drops` : "— drops"}
      </span>
      {(surfaceVersion !== undefined || correlationId !== undefined) && (
        <>
          <span className={styles.sep} aria-hidden>
            ·
          </span>
          {/* Provenance echo from the heartbeat — the surface version this line is
              pinned to (0 ⇒ live/unpinned) and the opening correlation id (0 ⇒ none). */}
          <span
            className={`num ${styles.item}`}
            data-testid="provenance-echo"
            title="Provenance echoed by the server heartbeat — surface version (sv) and correlation id (corr)"
          >
            {surfaceVersion !== undefined ? `sv ${surfaceVersion.toString()}` : ""}
            {surfaceVersion !== undefined && correlationId !== undefined ? " · " : ""}
            {correlationId !== undefined ? `corr ${correlationId.toString()}` : ""}
          </span>
        </>
      )}
    </>
  );
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
      <ServerObservabilityItems observability={app.stream.observability} />

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
      <span className={styles.sep} aria-hidden>
        ·
      </span>
      {/* Celer login-footer signature: a real build hash · UTC build time. */}
      <span
        className={`num ${styles.build}`}
        title="build provenance — git short SHA · UTC build time"
      >
        celnet {__CELNET_BUILD_HASH__} · {fmtBuildTime(__CELNET_BUILD_TIME__)}
      </span>
    </footer>
  );
}
