/**
 * StatusRibbon — the slim bottom ribbon (GUI-DESIGN §2): connection/stream
 * health, sequence, LP-in-competition count, last-look horizon, the global 24h
 * clock, and — prominently in the right cluster — the two headline TRADING
 * latencies: **avg quote** (mean time-to-quote) and **order p99** (execution-book
 * tail). These come from the shipped Latency/Ops analytics (`ListLatencyMetrics`),
 * polled on a low-frequency timer and gated on `view_analytics` (the same gate as
 * the Latency workspace) so a desk without analytics never sees a permanently-"—"
 * clutter and never hammers a server-denied RPC.
 *
 * It also surfaces the SERVER's own observability — distilled from the live
 * heartbeats (`StreamApi.observability`): the drain-side price-compute p99, the
 * exact ring conflation-drop count, and the surface-version / correlation
 * provenance echo. These are honest server measurements (the ribbon shows "—"
 * until the first beat lands — never a fabricated zero).
 */

import { useEffect, useState } from "react";
import { useApp } from "./AppContext";
import { useSecondClock } from "../hooks/useClock";
import type { ServerObservability } from "../hooks/useStreamSession";
import type { CelnetTransport } from "../data/transport";
import type { LatencyMetrics } from "../data/contract";
import { fmtClock, fmtLatencyNanos, fmtLatencyShort } from "../lib/format";
import styles from "./StatusRibbon.module.css";

/** How often the ribbon re-polls `ListLatencyMetrics` for the trading readout (ms). */
const TRADING_LATENCY_POLL_MS = 5_000;

/**
 * Stage `op` keys (server emit order) that best represent **time-to-quote**, in
 * preference order: the ESP tick→quote publish stage first (the dominant,
 * highest-volume quote path, literally tagged "tick→quote"), falling back to the
 * request-driven RFQ receive→respond stage when the ESP stage has no samples.
 */
const AVG_QUOTE_STAGE_OPS: readonly string[] = ["stream_publish", "rfq_respond"];

/**
 * Stage `op` keys for the **order / execution** tail, in preference order: the
 * ack→fill→book commit stage first, falling back to the quote→lift/accept stage.
 */
const ORDER_P99_STAGE_OPS: readonly string[] = ["book", "quote_accept"];

/** The two headline trading latencies (nanoseconds), or `null` when there is no data. */
export interface TradingLatency {
  /** Mean time-to-quote, ns — the mean of the first present {@link AVG_QUOTE_STAGE_OPS} stage. */
  avgQuoteNs: number | null;
  /** Order-latency P99, ns — the p99 of the first present {@link ORDER_P99_STAGE_OPS} stage. */
  orderP99Ns: number | null;
}

/**
 * The mean latency of the first stage (in `ops` preference order) that has samples.
 * A stage with `count === 0` reports zeros (a no-sample placeholder) and is skipped
 * so the ribbon never presents a fabricated zero. Returns `null` when none qualify.
 */
export function selectAvgQuoteNs(metrics: LatencyMetrics | null): number | null {
  return firstStageValue(metrics, AVG_QUOTE_STAGE_OPS, (s) => s.meanNs);
}

/** The p99 latency of the first present {@link ORDER_P99_STAGE_OPS} stage, or `null`. */
export function selectOrderP99Ns(metrics: LatencyMetrics | null): number | null {
  return firstStageValue(metrics, ORDER_P99_STAGE_OPS, (s) => s.p99Ns);
}

function firstStageValue(
  metrics: LatencyMetrics | null,
  ops: readonly string[],
  pick: (stage: LatencyMetrics["stages"][number]) => number,
): number | null {
  if (!metrics) return null;
  for (const op of ops) {
    const stage = metrics.stages.find((s) => s.op === op);
    if (stage && stage.count > 0) return pick(stage);
  }
  return null;
}

/**
 * Poll `ListLatencyMetrics` on a low-frequency timer while `enabled`, and derive the
 * two headline trading latencies. Any failure — RPC denial for a non-`view_analytics`
 * caller (belt-and-braces beyond the render gate), transport disconnect, or a decode
 * error — degrades to `null` ("—"), never throwing. Disabled ⇒ no poll, no data.
 */
function useTradingLatency(
  transport: Pick<CelnetTransport, "listLatencyMetrics">,
  enabled: boolean,
): TradingLatency {
  const [metrics, setMetrics] = useState<LatencyMetrics | null>(null);

  useEffect(() => {
    if (!enabled) {
      setMetrics(null);
      return;
    }
    let alive = true;
    const poll = (): void => {
      transport
        .listLatencyMetrics()
        .then((m) => {
          if (alive) setMetrics(m);
        })
        .catch(() => {
          if (alive) setMetrics(null);
        });
    };
    poll();
    const id = setInterval(poll, TRADING_LATENCY_POLL_MS);
    return () => {
      alive = false;
      clearInterval(id);
    };
  }, [transport, enabled]);

  return { avgQuoteNs: selectAvgQuoteNs(metrics), orderP99Ns: selectOrderP99Ns(metrics) };
}

/**
 * The trading-latency cluster — **avg quote** + **order p99** — as a pure component
 * over {@link TradingLatency} (no hooks), so it renders the SAME honest output for the
 * live WS transport and the offline mock, and is unit-testable with pinned values.
 * `null` renders "—" (never a fabricated zero). Trailing separator sits before the
 * transport badge that follows it in the ribbon's right cluster.
 */
export function TradingLatencyItems({ avgQuoteNs, orderP99Ns }: TradingLatency): React.ReactElement {
  return (
    <>
      <span
        className={`num ${styles.item} ${styles.trading}`}
        data-testid="avg-quote"
        title="Average time-to-quote — mean of the ESP tick→quote publish stage (falls back to RFQ receive→respond) from the live Latency/Ops analytics (ListLatencyMetrics)"
      >
        avg quote {avgQuoteNs === null ? "—" : fmtLatencyShort(avgQuoteNs)}
      </span>
      <span className={styles.sep} aria-hidden>
        ·
      </span>
      <span
        className={`num ${styles.item} ${styles.trading}`}
        data-testid="order-p99"
        title="Order-latency P99 — 99th-percentile of the ack→fill→book execution stage (falls back to quote→lift/accept) from the live Latency/Ops analytics (ListLatencyMetrics)"
      >
        order p99 {orderP99Ns === null ? "—" : fmtLatencyShort(orderP99Ns)}
      </span>
      <span className={styles.sep} aria-hidden>
        ·
      </span>
    </>
  );
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
      {/* Server price-compute p99 (drain-side HdrHistogram) — the server end of
          the latency budget, off the pinned hot core. */}
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

  // Gate the trading-latency readout on `view_analytics` (the same gate as the
  // Latency/Ops workspace) — held on EITHER asset admits (the server's cross-product
  // OR). Anonymous sessions run the permissive path (`can` ⇒ true), so the offline
  // mock and pre-sign-in shell still show it. A signed-in desk WITHOUT analytics
  // neither renders it nor polls the server-denied RPC.
  const canViewLatency =
    app.auth.can("view_analytics", "fx_options") || app.auth.can("view_analytics", "fixed_income");
  const latency = useTradingLatency(app.transport, canViewLatency);

  const healthy = app.stream.rows.filter((r) => r.health === "HEALTHY").length;
  const resyncing = app.stream.rows.filter((r) => r.health === "RESYNCING").length;
  const totalGaps = app.stream.rows.reduce((acc, r) => acc + r.gaps, 0);
  const allHealthy = resyncing === 0;

  return (
    // `data-transport-seam` carries the active transport identity (e.g. "mock/replay"
    // or "live ws://…") for machine checks (e2e live-vs-mock assertion) WITHOUT the
    // visible seam badge — the trader sees a clean ribbon; tooling still reads the seam.
    <footer className={styles.ribbon} data-transport-seam={app.transport.label}>
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

      {/* Headline TRADING latency (the point of the ribbon's right edge) — the
          right cluster, ahead of the clock. Trailing separator is emitted by the
          component. Only rendered for a `view_analytics` caller (see `canViewLatency`). */}
      {canViewLatency && (
        <TradingLatencyItems avgQuoteNs={latency.avgQuoteNs} orderP99Ns={latency.orderP99Ns} />
      )}

      <span className={`num ${styles.clock}`}>◷ {fmtClock(now)}</span>
      <span className={styles.sep} aria-hidden>
        ·
      </span>
      {/* Login-footer signature: a real build hash · UTC build time. */}
      <span
        className={`num ${styles.build}`}
        title="build provenance — git short SHA · UTC build time"
      >
        celnet {__CELNET_BUILD_HASH__} · {fmtBuildTime(__CELNET_BUILD_TIME__)}
      </span>
    </footer>
  );
}
