/**
 * StreamWorkspace — the RFS blotter, the RESTING STATE (GUI-DESIGN §4.2). A
 * living table of streaming two-ways multiplexed over one StreamSession. Only
 * changed numbers flash (calm under fire); per-row stream health is honest
 * (◉/◐/○ from real seq/resync state). Click a side to trade: the row carries
 * the maker's short-lived TradableTokens and a click sends Execute, surfacing a
 * typed Executed/StreamReject toast.
 *
 * Comparison note (positioning, not a product identifier): a streaming-first
 * (RFS) blotter as the resting state, the structural inversion of the
 * request-quote-per-click (RFQ) cadence common to incumbent options front-ends.
 *
 * HONESTY (CLAUDE.md rule 2): every plotted series is the row's OWN streamed
 * premium-mid history — the one trend the contract actually carries. The trend
 * mode is labelled "Premium"; the other modes (ATM vol, RR, BF, spot, …) are
 * shown DISABLED because they gate on a market-history feed the contract does
 * not expose — never a fabricated line (P0-4). The blotter's columns name their
 * units honestly: "Premium mid" in the conventions' premium units, an "impl σ"
 * implied-vol column, and a "Δ spot" delta column (P0-8).
 */

import { useMemo } from "react";
import { useApp } from "../app/AppContext";
import type { ScopeContext } from "../app/AppContext";
import { PriceTile } from "../components/PriceTile";
import { Sparkline, sparklineDirection, type SparklineDir } from "../components/Sparkline";
import { StatusBadge } from "../components/StatusBadge";
import { Button } from "../components/Button";
import type { StreamRow } from "../hooks/useStreamSession";
import { fmtPremiumPct, fmtSigned, fmtVol, premiumUnit } from "../lib/format";
import { TREND_MODES, DEFAULT_TREND_MODE } from "../lib/trend";
import { nowNanos } from "../hooks/useClock";
import styles from "./StreamWorkspace.module.css";

function tenorLabel(row: StreamRow): string {
  const t = row.instrument.tenor;
  switch (t.unit) {
    case "OVERNIGHT":
      return "ON";
    case "WEEKS":
      return `${t.count}W`;
    case "MONTHS":
      return `${t.count}M`;
    case "YEARS":
      return `${t.count}Y`;
  }
}

/**
 * Entitlement-ready scope filter (P0-6 seam). Today the principal is `grant-all`
 * and firm/desk/book are not subdivided, so this narrows ONLY when the scope has
 * drilled to a specific pair crumb (e.g. "EUR/USD") — a real, honest restriction
 * that a future entitlement principal will extend, with the blotter unchanged.
 */
function inScope(row: StreamRow, scope: ScopeContext): boolean {
  const tail = scope.path[scope.path.length - 1];
  if (!tail || tail.level !== "pair") return true;
  const label = `${row.instrument.pair.base}/${row.instrument.pair.quote}`;
  return label === tail.label;
}

/** The shared up/down glyph for the trend column (same rule as the sparkline). */
const TREND_GLYPH: Record<SparklineDir, string> = { up: "▲", down: "▼", flat: "▪" };

function BlotterRow({ row }: { row: StreamRow }): React.ReactElement {
  const app = useApp();
  const now = nowNanos();
  const sell = row.tradable.find((t) => t.side === "SELL");
  const buy = row.tradable.find((t) => t.side === "BUY");
  const tradable = row.health === "HEALTHY" && (sell?.validUntilNanos ?? 0n) > now;

  // ONE direction truth shared by the sparkline tint AND the trend glyph: the
  // net move across the visible premium-mid window (Sparkline.sparklineDirection).
  const trendDir = sparklineDirection(row.midHistory);
  const hasTrend = row.midHistory.length >= 2;

  // Premium-mid in this row's OWN premium units (each row carries its conventions).
  const unit = premiumUnit(row.conventions);
  const mid = (row.price.bid + row.price.offer) / 2;

  return (
    <div className={`${styles.row} ${row.health !== "HEALTHY" ? styles.dim : ""}`}>
      <span className={`num ${styles.pair}`}>
        {row.instrument.pair.base}/{row.instrument.pair.quote}
      </span>
      <span className={styles.structure}>{row.label}</span>
      <span className={`num ${styles.tenor}`}>{tenorLabel(row)}</span>

      <button
        className={`${styles.priceCell} ${styles.bidCell}`}
        disabled={!tradable || !sell}
        onClick={() => sell && app.stream.execute(row.subscriptionId, "SELL")}
        title={tradable ? "Hit the bid (SELL)" : "not tradable"}
      >
        <PriceTile value={row.price.bid} format={fmtPremiumPct} side="bid" />
      </button>
      <span className={`num ${styles.mid}`} title={`Premium mid (${unit})`}>
        {fmtPremiumPct(mid)}
      </span>
      <button
        className={`${styles.priceCell} ${styles.offerCell}`}
        disabled={!tradable || !buy}
        onClick={() => buy && app.stream.execute(row.subscriptionId, "BUY")}
        title={tradable ? "Lift the offer (BUY)" : "not tradable"}
      >
        <PriceTile value={row.price.offer} format={fmtPremiumPct} side="offer" showGlyph />
      </button>

      <span className={styles.spark}>
        {hasTrend ? (
          <>
            <span
              className={`num ${styles.trendGlyph} ${styles[`trend_${trendDir}`] ?? ""}`}
              aria-hidden="true"
            >
              {TREND_GLYPH[trendDir]}
            </span>
            <Sparkline
              values={row.midHistory}
              direction={trendDir}
              ariaLabel={`premium-mid trend, ${trendDir}`}
            />
          </>
        ) : (
          <span className={styles.noTrend} aria-label="no trend yet">
            —
          </span>
        )}
      </span>
      <span className={`num ${styles.greek}`} title="Spot delta (Δ)">
        {fmtSigned(row.greeks.deltaSpot, 3)}
      </span>
      <span className={`num ${styles.greek}`} title="Implied volatility (annualised, vol points)">
        {fmtVol(row.vol)}
      </span>
      <span className={styles.health}>
        <StatusBadge health={row.health} />
      </span>
    </div>
  );
}

/**
 * The honest trend-mode affordance: "Premium" is the one live series; every
 * other mode is shown DISABLED with a tooltip explaining it needs a market-
 * series feed the contract does not expose (P0-4). No fabricated trends.
 */
function TrendModeChips(): React.ReactElement {
  return (
    <div className={styles.trendModes} role="group" aria-label="trend series (premium is live)">
      <span className={styles.trendModesLabel}>Trend</span>
      {TREND_MODES.map((m) => {
        const active = m.id === DEFAULT_TREND_MODE;
        return (
          <span
            key={m.id}
            className={[
              styles.trendChip,
              active ? styles.trendChipActive : "",
              m.available ? "" : styles.trendChipDisabled,
            ]
              .filter(Boolean)
              .join(" ")}
            aria-disabled={m.available ? undefined : "true"}
            aria-current={active ? "true" : undefined}
            title={
              m.available
                ? `${m.label} — live streamed series`
                : `${m.label} — needs market-series feed`
            }
          >
            {m.label}
          </span>
        );
      })}
    </div>
  );
}

export function StreamWorkspace(): React.ReactElement {
  const app = useApp();
  // Entitlement-readiness: route the live rows through the firm-grant scope. It
  // narrows nothing today unless the scope has drilled to a specific pair crumb,
  // so a future desk/book principal can subdivide the universe without touching
  // this view (foundation P0-6 seam).
  const scope = app.scope;
  const rows = useMemo(
    () => app.stream.rows.filter((r) => inScope(r, scope)),
    [app.stream.rows, scope],
  );

  return (
    <div className={styles.wrap}>
      <div className={styles.headerRow}>
        <span className={styles.colPair}>Pair</span>
        <span className={styles.colStructure}>Structure</span>
        <span className={styles.colTenor}>Tenor</span>
        <span className={styles.colBid}>Bid</span>
        <span className={styles.colMid}>
          Premium mid
          <span className={styles.colUnit}>%</span>
        </span>
        <span className={styles.colOffer}>Offer</span>
        <span className={styles.colSpark}>
          Trend
          <span className={styles.colUnit}>premium</span>
        </span>
        <span className={styles.colGreek}>
          Δ
          <span className={styles.colUnit}>spot</span>
        </span>
        <span className={styles.colGreek}>
          σ
          <span className={styles.colUnit}>impl</span>
        </span>
        <span className={styles.colHealth} aria-label="stream health">
          ◉
        </span>
      </div>

      <div className={styles.body} role="grid" aria-label="streaming two-way markets">
        {rows.map((row) => (
          <BlotterRow key={row.subscriptionId.toString()} row={row} />
        ))}
      </div>

      <div className={styles.foot}>
        <Button variant="ghost" onClick={() => app.setPaletteOpen(true)}>
          + Subscribe (⌘K)
        </Button>
        <TrendModeChips />
        <span className={`num ${styles.footMeta}`}>
          Conflated 60Hz · click a side to trade · {app.stream.lpCount} LPs in competition
        </span>
      </div>

      {/* Click-to-trade outcome toasts (Executed / typed StreamReject). */}
      <div className={styles.toasts} aria-live="polite">
        {app.stream.toasts.map((t) => (
          <div
            key={t.id}
            className={`${styles.toast} ${t.kind === "executed" ? styles.toastOk : styles.toastReject}`}
            onClick={() => app.stream.dismissToast(t.id)}
          >
            <span className={styles.toastGlyph}>{t.kind === "executed" ? "✓" : "✕"}</span>
            <span>{t.text}</span>
          </div>
        ))}
      </div>
    </div>
  );
}
