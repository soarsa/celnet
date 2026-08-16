/**
 * HedgeFlowWorkspace — "which buckets need hedging, and where did the risk go?"
 *
 * Two things a rates desk asks all morning that no single screen answered:
 *
 *   1. **The bucket board.** Every risk book is drawn as a vessel filling toward its
 *      brim. The fill is not decoration — `utilization` IS `|netRisk| / threshold`,
 *      so a vessel at the brim is a book at its limit and an overflowing one is a
 *      breach. Worst-first, so the work queue is always leftmost.
 *
 *   2. **The flow strip.** Where fired risk actually landed: crossed internally
 *      against opposing flow, shed externally to an LP, or warehoused (still ours).
 *
 * Reads the SAME live sources the hedge monitor does — the advisory-intent stream
 * plus the provenance log — so the two screens can never disagree. Bands come from
 * the server verbatim; this component never decides what is red.
 *
 * Built mobile-first: the layout is a single auto-fitting grid, so the identical
 * markup serves a phone and a trading desk (see the stylesheet).
 */

import { useEffect, useMemo, useState } from "react";

import { useApp } from "../app/AppContext";
import type { HedgeProvenance, RiskBookRisk } from "../data/contract";
import type { HedgeBucket } from "../lib/hedgeBuckets";
import { bucketsFromRiskBooks, flowShares, flowTotals } from "../lib/hedgeBuckets";
import { formatDv01 } from "../lib/hedgeVehicle";
import styles from "./HedgeFlowWorkspace.module.css";

/** The server's band → the token the vessel is painted with. */
const BAND_COLOR: Record<string, string> = {
  breach: "var(--danger)",
  red: "var(--danger)",
  amber: "var(--warning)",
  green: "var(--accent-positive)",
};

/** An unknown band paints neutral rather than guessing at a severity. */
function bandColor(band: string): string {
  return BAND_COLOR[band.toLowerCase()] ?? "var(--text-tertiary)";
}

/**
 * One bucket, drawn as a vessel.
 *
 * The geometry is a trapezoid (wider at the brim, like a real bucket) plus a handle
 * arc. The fill is a rect clipped to the trapezoid and anchored at the base, so the
 * liquid takes the vessel's shape as it rises. An over-brim bucket draws full and
 * shows a spill mark — the drawing saturates, the NUMBER never does.
 */
function Vessel({ bucket }: { bucket: HedgeBucket }): React.ReactElement {
  const color = bandColor(bucket.band);
  const clipId = `bucket-clip-${bucket.book.replace(/[^a-zA-Z0-9_-]/g, "-")}`;
  // Body spans y 28→104; the liquid rises from the base.
  const top = 28;
  const bottom = 104;
  const height = (bottom - top) * bucket.fill;
  const y = bottom - height;
  return (
    <svg
      className={styles.vessel}
      viewBox="0 0 100 112"
      role="img"
      aria-label={`${bucket.book}: ${(bucket.utilization * 100).toFixed(0)}% of limit, ${bucket.band}`}
    >
      <defs>
        <clipPath id={clipId}>
          <path d="M14 28 L86 28 L74 104 L26 104 Z" />
        </clipPath>
      </defs>
      {/* handle */}
      <path
        d="M24 26 Q50 2 76 26"
        fill="none"
        stroke={color}
        strokeWidth="4"
        strokeLinecap="round"
        opacity="0.85"
      />
      {/* the liquid, clipped to the vessel so it takes the bucket's taper */}
      <rect
        x="0"
        y={y}
        width="100"
        height={height}
        fill={color}
        opacity="0.9"
        clipPath={`url(#${clipId})`}
      />
      {/* the vessel outline, drawn over the liquid */}
      <path
        d="M14 28 L86 28 L74 104 L26 104 Z"
        fill="none"
        stroke={color}
        strokeWidth="4"
        strokeLinejoin="round"
      />
      {/* spill mark — only when genuinely over the brim */}
      {bucket.overflow > 0 && (
        <path
          d="M86 28 q10 6 6 16"
          fill="none"
          stroke={color}
          strokeWidth="4"
          strokeLinecap="round"
        />
      )}
    </svg>
  );
}

export function HedgeFlowWorkspace(): React.ReactElement {
  const app = useApp();
  const [books, setBooks] = useState<RiskBookRisk[]>([]);
  const [provenance, setProvenance] = useState<HedgeProvenance[]>([]);

  /*
   * The board polls CURRENT risk and treats the intent stream as a refresh signal.
   *
   * Subscribing to intents alone does not work: the engine publishes one only when it
   * EVALUATES, so on a quiet desk the board sat on "waiting for the first tick" while
   * 21 hedges had already fired and every book carried live risk. `listRiskBookRisk()`
   * returns the state right now with each cap banded server-side, so the board is
   * populated on first paint. A slow 15s poll covers the case where nothing fires at
   * all; the intent push refreshes immediately when something does.
   */
  useEffect(() => {
    let cancelled = false;
    const refresh = (): void => {
      void app.transport
        .listRiskBookRisk()
        .then((b) => !cancelled && setBooks(b))
        .catch(() => undefined);
      void app.transport
        .listHedgeProvenance()
        .then((p) => !cancelled && setProvenance(p))
        .catch(() => undefined);
    };
    refresh();
    const timer = setInterval(refresh, 15_000);
    const dispose = app.transport.streamHedgeIntents(() => {
      if (!cancelled) refresh();
    });
    return () => {
      cancelled = true;
      clearInterval(timer);
      dispose();
    };
  }, [app.transport]);

  const buckets = useMemo(() => bucketsFromRiskBooks(books), [books]);
  const flow = useMemo(() => flowTotals(provenance), [provenance]);
  const shares = useMemo(() => flowShares(flow), [flow]);
  const needing = buckets.filter((b) => b.needsHedge).length;

  const legs = [
    {
      key: "crossed",
      label: "Crossed internally",
      value: flow.crossed,
      share: shares.crossed,
      cls: styles.fillCrossed,
      note: "netted against opposing flow",
    },
    {
      key: "hedged",
      label: "Hedged externally",
      value: flow.hedged,
      share: shares.hedged,
      cls: styles.fillHedged,
      note: "shed to an LP",
    },
    {
      key: "warehoused",
      label: "Warehoused",
      value: flow.warehoused,
      share: shares.warehoused,
      cls: styles.fillWarehoused,
      note: "still our risk",
    },
  ];

  return (
    <div className={styles.root} data-testid="hedge-flow-workspace">
      <header className={styles.header}>
        <h2 className={styles.title}>Hedge flow</h2>
        <p className={styles.subtitle}>
          Live risk buckets and where fired hedges sent the risk
        </p>
        <span
          className={`${styles.attention} ${needing === 0 ? styles.attentionClear : ""}`}
          data-testid="hedge-flow-attention"
        >
          {needing === 0 ? "no bucket at limit" : `${needing} need hedging`}
        </span>
      </header>

      {buckets.length === 0 ? (
        <p className={styles.empty} data-testid="hedge-flow-empty">
          No risk book reports a computable cap yet…
        </p>
      ) : (
        <ul className={styles.buckets} data-testid="hedge-buckets">
          {buckets.map((b) => (
            <li
              key={b.book}
              className={`${styles.bucket} ${b.needsHedge ? styles.bucketNeedsHedge : ""}`}
              data-testid={`hedge-bucket-${b.book}`}
            >
              <Vessel bucket={b} />
              <span className={styles.pct}>{(b.utilization * 100).toFixed(0)}%</span>
              <span className={styles.bookName} title={b.book}>
                {b.book}
              </span>
              <span className={styles.bandLabel}>{b.band}</span>
              <span className={styles.risk}>{formatDv01(b.netRisk)}</span>
            </li>
          ))}
        </ul>
      )}

      <div className={styles.flow} data-testid="hedge-flow-legs">
        {legs.map((leg) => (
          <div key={leg.key} className={styles.leg} data-testid={`hedge-flow-${leg.key}`}>
            <span className={styles.legLabel}>{leg.label}</span>
            <span className={styles.legValue}>{formatDv01(leg.value)}</span>
            <span className={styles.legBar}>
              <span
                className={`${styles.legFill} ${leg.cls}`}
                style={{ width: `${leg.share * 100}%` }}
              />
            </span>
            <span className={styles.legNote}>{leg.note}</span>
          </div>
        ))}
      </div>
      <p className={styles.legNote}>
        {flow.fires === 0
          ? "No hedges have fired yet — advisory (dry-run) fires are excluded."
          : `From ${flow.fires} fired hedge${flow.fires === 1 ? "" : "s"} · advisory fires excluded.`}
      </p>
    </div>
  );
}
