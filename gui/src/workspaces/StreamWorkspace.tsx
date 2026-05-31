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
 * HONESTY (CLAUDE.md rule 2): PREMIUM plots the row's OWN streamed premium-mid
 * history; the market-observable modes (ATM vol, RR, BF, spot, forward) plot a
 * REAL series streamed from the contract's market-series feed
 * (`MarketSeriesSubscribe`, served by celnet-server) — never a fabricated line.
 * VEGA/PNL stay DISABLED (they need the position-fact store, a later phase). The
 * active mode's label + unit are always shown on the trend column header and the
 * tile. The blotter's columns name their units honestly: "Premium mid" in the
 * conventions' premium units, an "impl σ" implied-vol column, and a "Δ spot"
 * delta column (P0-8).
 */

import { useMemo, useState } from "react";
import { useApp } from "../app/AppContext";
import type { ScopeContext } from "../app/AppContext";
import { PriceTile } from "../components/PriceTile";
import { Sparkline, sparklineDirection, type SparklineDir } from "../components/Sparkline";
import { StatusBadge } from "../components/StatusBadge";
import { Button } from "../components/Button";
import { ownerLabel, type StreamRow } from "../hooks/useStreamSession";
import { useTrendSeries, type TrendRowKey, type TrendSeries } from "../hooks/useTrendSeries";
import { fmtPremiumPct, fmtRate, fmtSigned, fmtVol, fmtVolPoint, premiumUnit } from "../lib/format";
import {
  TREND_MODES,
  DEFAULT_TREND_MODE,
  trendModeSpec,
  type TrendMode,
  type TrendUnit,
} from "../lib/trend";
import { nowNanos } from "../hooks/useClock";
import styles from "./StreamWorkspace.module.css";

/** Format a trend value in its natural unit (for the tile's numeric readout). */
function fmtTrendValue(unit: TrendUnit, value: number): string {
  switch (unit) {
    case "premium":
      return fmtPremiumPct(value);
    case "vol":
      return fmtVolPoint(value);
    case "rate":
      return fmtRate(value);
    case "vega":
    case "pnl":
      return value.toFixed(2);
  }
}

/** The short unit caption shown under the trend column header for a mode. */
function trendUnitCaption(mode: TrendMode): string {
  switch (mode) {
    case "PREMIUM":
      return "premium";
    case "ATM_VOL":
      return "ATM vol";
    case "RR":
      return "25Δ RR";
    case "BF":
      return "25Δ BF";
    case "SPOT":
      return "spot";
    case "FORWARD":
      return "fwd";
    case "VEGA":
      return "vega";
    case "PNL":
      return "P&L";
  }
}

function tenorLabel(row: StreamRow): string {
  const t = row.instrument.tenor;
  switch (t.unit) {
    case "OVERNIGHT":
      return "ON";
    case "TOM_NEXT":
      return "TN";
    case "SPOT_NEXT":
      return "SN";
    case "WEEKS":
      return `${t.count}W`;
    case "MONTHS":
      return `${t.count}M`;
    case "YEARS":
      return `${t.count}Y`;
    case "IMM":
      return `${t.count}IMM`;
    case "BROKEN_DATE":
      return t.brokenDate
        ? `${t.brokenDate.year}-${String(t.brokenDate.month).padStart(2, "0")}-${String(t.brokenDate.day).padStart(2, "0")}`
        : "broken";
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

function BlotterRow({
  row,
  mode,
  trend,
}: {
  row: StreamRow;
  mode: TrendMode;
  /** The row's streamed market-series (non-PREMIUM modes), or undefined. */
  trend: TrendSeries | undefined;
}): React.ReactElement {
  const app = useApp();
  const now = nowNanos();
  const sell = row.tradable.find((t) => t.side === "SELL");
  const buy = row.tradable.find((t) => t.side === "BUY");
  const tradable = row.health === "HEALTHY" && (sell?.validUntilNanos ?? 0n) > now;

  const spec = trendModeSpec(mode);
  // PREMIUM plots the row's OWN streamed premium mid (always live); a market-
  // observable mode plots the REAL streamed market-series for this row; VEGA/PNL
  // are gated (no series). Each path is honest — no fabricated values.
  const trendValues = mode === "PREMIUM" ? row.midHistory : (trend?.values ?? []);
  // ONE direction truth shared by the sparkline tint AND the trend glyph: the
  // net move across the visible window (Sparkline.sparklineDirection).
  const trendDir = sparklineDirection(trendValues);
  const hasTrend = trendValues.length >= 2;
  const trendLatest =
    mode === "PREMIUM"
      ? trendValues.length > 0
        ? trendValues[trendValues.length - 1]
        : undefined
      : trend?.latest;

  // Premium-mid in this row's OWN premium units (each row carries its conventions).
  const unit = premiumUnit(row.conventions);
  const mid = (row.price.bid + row.price.offer) / 2;

  return (
    <div className={`${styles.row} ${row.health !== "HEALTHY" ? styles.dim : ""}`}>
      <span className={`num ${styles.pair}`}>
        {row.instrument.pair.base}/{row.instrument.pair.quote}
      </span>
      <span className={styles.structure}>
        {row.label}
        {/* Honest attribution: show the maker/owner only when the wire carries it
            (engine-quoted edge flow is the auto-pricer); nothing when absent. */}
        {ownerLabel(row.attribution?.quotedBy) && (
          <span className={styles.owner} title="Quoted by">
            {ownerLabel(row.attribution?.quotedBy)}
          </span>
        )}
      </span>
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
        {!spec.available ? (
          <span className={styles.noTrend} aria-label={`${spec.label} not available`}>
            —
          </span>
        ) : hasTrend ? (
          <>
            <span
              className={`num ${styles.trendGlyph} ${styles[`trend_${trendDir}`] ?? ""}`}
              aria-hidden="true"
            >
              {TREND_GLYPH[trendDir]}
            </span>
            <Sparkline
              values={trendValues}
              direction={trendDir}
              ariaLabel={`${spec.label} trend, ${trendDir}`}
            />
            {trendLatest !== undefined && (
              <span className={`num ${styles.trendValue}`} title={`${spec.label} (${trendUnitCaption(mode)})`}>
                {fmtTrendValue(spec.unit, trendLatest)}
              </span>
            )}
          </>
        ) : (
          <span className={styles.noTrend} aria-label="awaiting series">
            …
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
 * The trend-mode selector. PREMIUM and the market-observable modes (ATM vol / RR /
 * BF / spot / forward) are SELECTABLE — clicking one re-plots every row's trend
 * column from the real streamed series for that observable. VEGA/PNL are shown
 * DISABLED (they need the position-fact store, a later phase). No fabricated trends.
 */
function TrendModeChips({
  mode,
  onSelect,
}: {
  mode: TrendMode;
  onSelect: (m: TrendMode) => void;
}): React.ReactElement {
  return (
    <div className={styles.trendModes} role="group" aria-label="trend series mode">
      <span className={styles.trendModesLabel}>Trend</span>
      {TREND_MODES.map((m) => {
        const active = m.id === mode;
        return (
          <button
            key={m.id}
            type="button"
            className={[
              styles.trendChip,
              active ? styles.trendChipActive : "",
              m.available ? "" : styles.trendChipDisabled,
            ]
              .filter(Boolean)
              .join(" ")}
            disabled={!m.available}
            aria-pressed={active}
            onClick={() => m.available && onSelect(m.id)}
            title={
              m.available
                ? `${m.label} — live streamed series`
                : `${m.label} — needs the position-fact store (later phase)`
            }
          >
            {m.label}
          </button>
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

  // The active trend-column mode. PREMIUM uses the row's own streamed mid; the
  // market-observable modes stream the contract's market-series feed (below).
  const [trendMode, setTrendMode] = useState<TrendMode>(DEFAULT_TREND_MODE);

  // The per-row keys for the market-series subscriptions (pair + pillar tenor).
  const trendKeys = useMemo<TrendRowKey[]>(
    () =>
      rows.map((r) => ({
        subscriptionId: r.subscriptionId,
        pair: r.instrument.pair,
        tenorYears: r.instrument.expiryYears,
      })),
    [rows],
  );
  // The live streamed series per row for the active observable (empty for PREMIUM
  // and the gated modes — the row falls back to its own premium history).
  const trendSeries = useTrendSeries(app.transport, trendMode, trendKeys);
  const trendUnitLabel = trendUnitCaption(trendMode);

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
          <span className={styles.colUnit}>{trendUnitLabel}</span>
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
          <BlotterRow
            key={row.subscriptionId.toString()}
            row={row}
            mode={trendMode}
            trend={trendSeries.get(row.subscriptionId)}
          />
        ))}
      </div>

      <div className={styles.foot}>
        <Button variant="ghost" onClick={() => app.setPaletteOpen(true)}>
          + Subscribe (⌘K)
        </Button>
        <TrendModeChips mode={trendMode} onSelect={setTrendMode} />
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
