/**
 * MobileClientBlotter — received CLIENT deals as stacked, tappable row-cards (time,
 * counterparty, product · tenor, notional, side, routed portfolio, and the
 * internalise/warehoused/B2B badge). Tap a card to expand the execution detail.
 *
 * Read-only over `listDeals` rows already filtered to the active asset. Capped with
 * an honest "Show more" (never a silent truncation).
 */

import { useState } from "react";

import type { CapabilityAsset, Deal } from "../../data/contract";
import { fmtClock, fmtCompact, sideVerb } from "../../lib/format";
import { fmtEdgeBps, hedgeBandLabel, internaliseLabel } from "../../lib/internalise";
import { MobileEmpty, MobileError, MobileSkeleton } from "./MobileStates";
import styles from "./MobileStatusApp.module.css";

/** Rows shown before the first "Show more" (keeps the phone from rendering thousands). */
const PAGE = 50;

/** The product · tenor descriptor for a deal (BOND shows its security name). */
function productLabel(deal: Deal): string {
  if (deal.productKind === "BOND") {
    return deal.bondDisplayName || deal.bondSecurityId || "BOND";
  }
  const tenor = deal.instrument.tenorYears;
  const tenorLabel = typeof tenor === "number" ? `${tenor}Y` : "";
  return tenorLabel ? `${deal.productKind} · ${tenorLabel}` : deal.productKind;
}

function ClientRow({
  deal,
  bookNames,
}: {
  deal: Deal;
  bookNames: ReadonlyMap<string, string>;
}): React.ReactElement {
  const [open, setOpen] = useState(false);
  const inl = deal.internalise;
  const bookName = deal.riskBookId ? bookNames.get(deal.riskBookId) ?? deal.riskBookId : undefined;
  const sideTone = deal.side === "BUY" ? "buy" : deal.side === "SELL" ? "sell" : "two";
  return (
    <article className={styles.row} data-open={open || undefined}>
      <button
        type="button"
        className={styles.rowBtn}
        aria-expanded={open}
        onClick={() => setOpen((o) => !o)}
      >
        <span className={styles.rowTime}>{fmtClock(deal.executedAtNanos)}</span>
        <span className={styles.rowMain}>
          <span className={styles.rowTitle}>{deal.counterparty}</span>
          <span className={styles.rowSub}>
            <span className={styles.rowProduct}>{productLabel(deal)}</span>
            {inl && (
              <span className={styles.inlBadge} data-band={inl.hedgeBand}>
                {internaliseLabel(inl)}
              </span>
            )}
          </span>
        </span>
        <span className={styles.rowRight}>
          <span className={styles.sideBadge} data-side={sideTone}>
            {sideVerb(deal.side)}
          </span>
          <span className={styles.rowNotional}>{fmtCompact(deal.notional)}</span>
        </span>
      </button>
      {open && (
        <dl className={styles.rowDetail}>
          <div>
            <dt>Level</dt>
            <dd className={styles.num}>{deal.price.toFixed(4)}</dd>
          </div>
          <div>
            <dt>Portfolio</dt>
            <dd>{bookName ?? "—"}</dd>
          </div>
          <div>
            <dt>Trader</dt>
            <dd>{deal.trader || "—"}</dd>
          </div>
          {inl && (
            <div className={styles.detailWide}>
              <dt>Disposition</dt>
              <dd>
                <span className={styles.inlBadge} data-band={inl.hedgeBand}>
                  {internaliseLabel(inl)}
                </span>{" "}
                edge {fmtEdgeBps(inl.edgeBps)} · {hedgeBandLabel(inl.hedgeBand)} band
              </dd>
            </div>
          )}
        </dl>
      )}
    </article>
  );
}

export interface MobileClientBlotterProps {
  deals: readonly Deal[];
  bookNames: ReadonlyMap<string, string>;
  isLoading: boolean;
  error: unknown;
  asset: CapabilityAsset;
}

export function MobileClientBlotter({
  deals,
  bookNames,
  isLoading,
  error,
  asset,
}: MobileClientBlotterProps): React.ReactElement {
  const [limit, setLimit] = useState(PAGE);
  if (isLoading && deals.length === 0) return <MobileSkeleton rows={5} />;
  if (error && deals.length === 0) return <MobileError message="Couldn't load client deals." />;
  if (deals.length === 0) {
    return (
      <MobileEmpty
        message={
          asset === "fx_options" ? "No FX Options client deals yet." : "No client deals yet."
        }
      />
    );
  }

  // Newest first, then cap.
  const sorted = [...deals].sort((a, b) =>
    a.executedAtNanos < b.executedAtNanos ? 1 : a.executedAtNanos > b.executedAtNanos ? -1 : 0,
  );
  const shown = sorted.slice(0, limit);
  const remaining = sorted.length - shown.length;

  return (
    <div className={styles.board}>
      <ul className={styles.rowList}>
        {shown.map((deal) => (
          <li key={deal.dealId}>
            <ClientRow deal={deal} bookNames={bookNames} />
          </li>
        ))}
      </ul>
      {remaining > 0 && (
        <button
          type="button"
          className={styles.moreBtn}
          onClick={() => setLimit((n) => n + PAGE)}
        >
          Show {Math.min(PAGE, remaining)} more ({remaining} hidden)
        </button>
      )}
    </div>
  );
}
