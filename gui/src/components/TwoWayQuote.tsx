/**
 * TwoWayQuote — bid | mid | offer with a last-look ring and convention chips
 * (GUI-DESIGN §3.5). A reusable, side-tinted two-way market face used wherever a
 * compact tradable two-way is shown (e.g. an inline strategy two-way, a hover
 * preview). Each side flashes on change via PriceTile; the ring depletes against
 * the line's validity deadline. Click a side to trade if handlers are supplied.
 */

import type { Conventions, TwoWayPrice } from "../data/contract";
import { fmtPremiumPct } from "../lib/format";
import { ConventionRow } from "./ConventionChip";
import { LastLookRing } from "./LastLookRing";
import { PriceTile } from "./PriceTile";
import styles from "./TwoWayQuote.module.css";

export interface TwoWayQuoteProps {
  price: TwoWayPrice;
  conventions: Conventions;
  /** Optional last-look deadline (nanoseconds since epoch) + window length (s). */
  validUntilNanos?: bigint;
  windowSeconds?: number;
  onHitBid?: () => void;
  onLiftOffer?: () => void;
  size?: "display" | "headline" | "callout";
}

export function TwoWayQuote({
  price,
  conventions,
  validUntilNanos,
  windowSeconds = 8,
  onHitBid,
  onLiftOffer,
  size = "headline",
}: TwoWayQuoteProps): React.ReactElement {
  const mid = (price.bid + price.offer) / 2;
  return (
    <div className={styles.wrap}>
      <div className={styles.market}>
        <button
          className={`${styles.side} ${styles.bid}`}
          disabled={!onHitBid}
          onClick={onHitBid}
          title={onHitBid ? "Hit the bid (SELL)" : undefined}
        >
          <span className={styles.label}>BID</span>
          <PriceTile value={price.bid} format={fmtPremiumPct} side="bid" size={size} />
        </button>
        <div className={styles.midCol}>
          <span className={styles.label}>MID</span>
          <PriceTile value={mid} format={fmtPremiumPct} size={size} />
        </div>
        <button
          className={`${styles.side} ${styles.offer}`}
          disabled={!onLiftOffer}
          onClick={onLiftOffer}
          title={onLiftOffer ? "Lift the offer (BUY)" : undefined}
        >
          <span className={styles.label}>OFFER</span>
          <PriceTile value={price.offer} format={fmtPremiumPct} side="offer" size={size} showGlyph />
        </button>
      </div>
      <div className={styles.foot}>
        <ConventionRow conventions={conventions} />
        {validUntilNanos !== undefined && (
          <LastLookRing validUntilNanos={validUntilNanos} windowSeconds={windowSeconds} />
        )}
      </div>
    </div>
  );
}
