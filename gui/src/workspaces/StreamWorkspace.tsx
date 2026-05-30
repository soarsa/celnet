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
 */

import { useApp } from "../app/AppContext";
import { PriceTile } from "../components/PriceTile";
import { Sparkline } from "../components/Sparkline";
import { StatusBadge } from "../components/StatusBadge";
import { Button } from "../components/Button";
import type { StreamRow } from "../hooks/useStreamSession";
import { fmtPremiumPct, fmtSigned, fmtVol } from "../lib/format";
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

function BlotterRow({ row }: { row: StreamRow }): React.ReactElement {
  const app = useApp();
  const now = nowNanos();
  const sell = row.tradable.find((t) => t.side === "SELL");
  const buy = row.tradable.find((t) => t.side === "BUY");
  const tradable = row.health === "HEALTHY" && (sell?.validUntilNanos ?? 0n) > now;

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
        onClick={() => sell && app.stream.execute(row.subscriptionId, sell.token)}
        title={tradable ? "Hit the bid (SELL)" : "not tradable"}
      >
        <PriceTile value={row.price.bid} format={fmtPremiumPct} side="bid" />
      </button>
      <span className={`num ${styles.mid}`}>{fmtPremiumPct((row.price.bid + row.price.offer) / 2)}</span>
      <button
        className={`${styles.priceCell} ${styles.offerCell}`}
        disabled={!tradable || !buy}
        onClick={() => buy && app.stream.execute(row.subscriptionId, buy.token)}
        title={tradable ? "Lift the offer (BUY)" : "not tradable"}
      >
        <PriceTile value={row.price.offer} format={fmtPremiumPct} side="offer" showGlyph />
      </button>

      <span className={styles.spark}>
        <Sparkline values={row.midHistory} />
      </span>
      <span className={`num ${styles.greek}`}>{fmtSigned(row.greeks.deltaSpot, 3)}</span>
      <span className={`num ${styles.greek}`}>{fmtVol(row.vol)}</span>
      <span className={styles.health}>
        <StatusBadge health={row.health} />
      </span>
    </div>
  );
}

export function StreamWorkspace(): React.ReactElement {
  const app = useApp();
  return (
    <div className={styles.wrap}>
      <div className={styles.headerRow}>
        <span className={styles.colPair}>Pair</span>
        <span className={styles.colStructure}>Structure</span>
        <span className={styles.colTenor}>Tenor</span>
        <span className={styles.colBid}>Bid</span>
        <span className={styles.colMid}>Mid</span>
        <span className={styles.colOffer}>Offer</span>
        <span className={styles.colSpark}>Trend</span>
        <span className={styles.colGreek}>Δ</span>
        <span className={styles.colGreek}>σ</span>
        <span className={styles.colHealth} aria-label="stream health">
          ◉
        </span>
      </div>

      <div className={styles.body} role="grid" aria-label="streaming two-way markets">
        {app.stream.rows.map((row) => (
          <BlotterRow key={row.subscriptionId.toString()} row={row} />
        ))}
      </div>

      <div className={styles.foot}>
        <Button variant="ghost" onClick={() => app.setPaletteOpen(true)}>
          + Subscribe (⌘K)
        </Button>
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
