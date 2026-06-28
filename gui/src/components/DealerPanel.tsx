/**
 * DealerPanel — the ranked multi-dealer (RFQ-to-many) quote panel: one row per
 * responding LP from a `MultiDealerQuote` frame, rendered IN FRAME ORDER (the
 * server's deterministic audit order — the panel never re-sorts), with the touch
 * winners (`bestBidLpId`/`bestOfferLpId`) highlighted on their winning side.
 * Clicking a row's bid/offer books exactly that pinned dealer line by
 * `(quoteId, lpId)`; each row depletes its OWN last-look ring against the line's
 * `validUntilNanos`, and an expired row is disabled with an honest reason — the
 * trader can never trade an expired window.
 *
 * a11y: a real `<table>` (APG data-grid-with-actions pattern) — column headers
 * via `<th scope="col">`, the LP identity as the row header (`<th scope="row">`),
 * and the book actions as native buttons inside the price cells. Winner
 * highlighting carries a text badge ("best"), never colour alone.
 */

import type { DealerQuote, MultiDealerQuote } from "../data/contract";
import { useCountdownClock } from "../hooks/useClock";
import { fmtPremiumPct, secondsUntil } from "../lib/format";
import { LastLookRing } from "./LastLookRing";
import styles from "./DealerPanel.module.css";

export interface DealerPanelProps {
  panel: MultiDealerQuote;
  /** The full last-look window length in seconds (the ring depletion base). */
  windowSeconds?: number;
  /** Book one dealer line: SELL hits its bid, BUY lifts its offer. */
  onBook: (quoteId: bigint, lpId: string, side: "BUY" | "SELL") => void;
  /**
   * When set, the book buttons are disabled regardless of expiry — the signed-in
   * user lacks the execute capability for this surface. The row stays VISIBLE
   * (never hidden) with {@link bookDisabledTitle} explaining why.
   */
  bookDisabled?: boolean;
  /** The explanatory tooltip for a capability-disabled book button. */
  bookDisabledTitle?: string;
}

export function DealerPanel({
  panel,
  windowSeconds = 8,
  onBook,
  bookDisabled = false,
  bookDisabledTitle,
}: DealerPanelProps): React.ReactElement {
  // One shared countdown clock drives every row's ring + expiry gate.
  const now = useCountdownClock(true);
  return (
    <div className={styles.wrap}>
      <table className={styles.table} aria-label="multi-dealer quote panel">
        <thead>
          <tr>
            <th scope="col" className={styles.colHead}>
              LP
            </th>
            <th scope="col" className={styles.colHead}>
              Bid
            </th>
            <th scope="col" className={styles.colHead}>
              Offer
            </th>
            <th scope="col" className={styles.colHead}>
              Last-look
            </th>
          </tr>
        </thead>
        <tbody>
          {panel.dealers.map((d) => (
            <DealerRow
              key={d.lpId}
              dealer={d}
              quoteId={panel.quoteId}
              bestBid={panel.bestBidLpId === d.lpId}
              bestOffer={panel.bestOfferLpId === d.lpId}
              expired={d.validUntilNanos <= now}
              remainingSeconds={secondsUntil(d.validUntilNanos, now)}
              windowSeconds={windowSeconds}
              onBook={onBook}
              bookDisabled={bookDisabled}
              bookDisabledTitle={bookDisabledTitle}
            />
          ))}
        </tbody>
      </table>
      <p className={styles.note}>
        {panel.dealers.length} LP line{panel.dealers.length === 1 ? "" : "s"} ranked
        best-bid / best-offer · book a line to trade exactly that dealer's price.
      </p>
    </div>
  );
}

function DealerRow(props: {
  dealer: DealerQuote;
  quoteId: bigint;
  bestBid: boolean;
  bestOffer: boolean;
  expired: boolean;
  remainingSeconds: number;
  windowSeconds: number;
  onBook: (quoteId: bigint, lpId: string, side: "BUY" | "SELL") => void;
  bookDisabled: boolean;
  bookDisabledTitle: string | undefined;
}): React.ReactElement {
  const { dealer, quoteId, bestBid, bestOffer, expired, windowSeconds, onBook } = props;
  const { bookDisabled, bookDisabledTitle } = props;
  // A book button is dead either because the line expired OR the signed-in user
  // lacks the execute capability; the tooltip explains whichever applies.
  const bidDisabled = expired || bookDisabled;
  const offerDisabled = expired || bookDisabled;
  const denyTitle = bookDisabled ? bookDisabledTitle : undefined;
  return (
    <tr className={expired ? styles.expiredRow : undefined}>
      <th scope="row" className={`num ${styles.lp}`}>
        {dealer.lpId}
      </th>
      <td className={`${styles.priceCell} ${bestBid ? styles.bestBid : ""}`}>
        <button
          className={`${styles.book} ${styles.bid}`}
          disabled={bidDisabled}
          onClick={() => onBook(quoteId, dealer.lpId, "SELL")}
          aria-label={`sell to ${dealer.lpId} at ${fmtPremiumPct(dealer.price.bid)}${
            bestBid ? " — best bid" : ""
          }`}
          title={
            denyTitle ??
            (expired ? "line expired — re-request the panel" : "Hit this bid (SELL)")
          }
        >
          <span className="num">{fmtPremiumPct(dealer.price.bid)}</span>
          {bestBid && <span className={styles.best}>best</span>}
        </button>
      </td>
      <td className={`${styles.priceCell} ${bestOffer ? styles.bestOffer : ""}`}>
        <button
          className={`${styles.book} ${styles.offer}`}
          disabled={offerDisabled}
          onClick={() => onBook(quoteId, dealer.lpId, "BUY")}
          aria-label={`buy from ${dealer.lpId} at ${fmtPremiumPct(dealer.price.offer)}${
            bestOffer ? " — best offer" : ""
          }`}
          title={
            denyTitle ??
            (expired ? "line expired — re-request the panel" : "Lift this offer (BUY)")
          }
        >
          <span className="num">{fmtPremiumPct(dealer.price.offer)}</span>
          {bestOffer && <span className={styles.best}>best</span>}
        </button>
      </td>
      <td className={styles.validityCell}>
        {expired ? (
          <span className={styles.expiredReason}>expired — re-request</span>
        ) : (
          <LastLookRing
            validUntilNanos={dealer.validUntilNanos}
            windowSeconds={windowSeconds}
            label={`${dealer.lpId} last-look`}
          />
        )}
      </td>
    </tr>
  );
}
