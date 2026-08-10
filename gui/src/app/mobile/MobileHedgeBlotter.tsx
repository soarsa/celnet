/**
 * MobileHedgeBlotter — fired hedges as stacked, tappable row-cards (time, portfolio,
 * action/venue, hedged size, internal-cross vs external-LP, advisory flag). Tap a
 * card to expand the slippage / mid / residual / policy detail.
 *
 * Read-only over `listHedgeProvenance` (the immutable fired-hedge audit trail),
 * already filtered to the active asset. Capped with an honest "Show more".
 */

import { useState } from "react";

import type { CapabilityAsset, HedgeProvenance } from "../../data/contract";
import { fmtCompact } from "../../lib/format";
import { MobileEmpty, MobileError, MobileSkeleton } from "./MobileStates";
import styles from "./MobileStatusApp.module.css";

const PAGE = 50;

/** epoch-millis → local 24h clock (HedgeProvenance.firedAt is millis, not nanos). */
function clockMs(firedAt: number): string {
  return new Date(firedAt).toLocaleTimeString("en-GB", { hour12: false });
}

/** The venue/action label: winning LP, internal cross, or a no-trade action. */
function venueLabel(h: HedgeProvenance): string {
  if (h.lpWon) return `LP ${h.lpWon}`;
  if (h.externalHedged > 0) return "External";
  if (h.internalCrossed > 0) return "Internal cross";
  return "No trade";
}

/** Whether the hedge shed risk externally (an LP win or external volume). */
function isExternal(h: HedgeProvenance): boolean {
  return Boolean(h.lpWon) || h.externalHedged > 0;
}

function HedgeRow({
  hedge,
  bookNames,
}: {
  hedge: HedgeProvenance;
  bookNames: ReadonlyMap<string, string>;
}): React.ReactElement {
  const [open, setOpen] = useState(false);
  const book = bookNames.get(hedge.book) ?? hedge.book;
  const hedged = hedge.internalCrossed + hedge.externalHedged;
  const external = isExternal(hedge);
  return (
    <article className={styles.row} data-open={open || undefined}>
      <button
        type="button"
        className={styles.rowBtn}
        aria-expanded={open}
        onClick={() => setOpen((o) => !o)}
      >
        <span className={styles.rowTime}>{clockMs(hedge.firedAt)}</span>
        <span className={styles.rowMain}>
          <span className={styles.rowTitle}>{book}</span>
          <span className={styles.rowSub}>
            {venueLabel(hedge)}
            {hedge.advisory && <span className={styles.advisory}>advisory</span>}
          </span>
        </span>
        <span className={styles.rowRight}>
          <span className={styles.sideBadge} data-side={external ? "sell" : "two"}>
            {external ? "External" : "Internal"}
          </span>
          <span className={styles.rowNotional}>{fmtCompact(hedged)}</span>
        </span>
      </button>
      {open && (
        <dl className={styles.rowDetail}>
          <div>
            <dt>Instrument</dt>
            <dd>{hedge.instrument}</dd>
          </div>
          <div>
            <dt>Band</dt>
            <dd>
              <span className={styles.inlBadge} data-band={hedge.band}>
                {hedge.band}
              </span>
            </dd>
          </div>
          <div>
            <dt>Internal cross</dt>
            <dd className={styles.num}>{fmtCompact(hedge.internalCrossed)}</dd>
          </div>
          <div>
            <dt>External</dt>
            <dd className={styles.num}>{fmtCompact(hedge.externalHedged)}</dd>
          </div>
          <div>
            <dt>Residual</dt>
            <dd className={styles.num}>{fmtCompact(hedge.residual)}</dd>
          </div>
          <div>
            <dt>Slippage</dt>
            <dd className={styles.num}>{hedge.slippageBp.toFixed(2)} bp</dd>
          </div>
        </dl>
      )}
    </article>
  );
}

export interface MobileHedgeBlotterProps {
  hedges: readonly HedgeProvenance[];
  bookNames: ReadonlyMap<string, string>;
  isLoading: boolean;
  error: unknown;
  asset: CapabilityAsset;
}

export function MobileHedgeBlotter({
  hedges,
  bookNames,
  isLoading,
  error,
  asset,
}: MobileHedgeBlotterProps): React.ReactElement {
  const [limit, setLimit] = useState(PAGE);
  if (isLoading && hedges.length === 0) return <MobileSkeleton rows={5} />;
  if (error && hedges.length === 0) return <MobileError message="Couldn't load hedges." />;
  if (hedges.length === 0) {
    return (
      <MobileEmpty
        message={
          asset === "fx_options"
            ? "No FX Options hedges (hedging runs on fixed income)."
            : "No hedges fired yet."
        }
      />
    );
  }

  const sorted = [...hedges].sort((a, b) => b.firedAt - a.firedAt);
  const shown = sorted.slice(0, limit);
  const remaining = sorted.length - shown.length;

  return (
    <div className={styles.board}>
      <ul className={styles.rowList}>
        {shown.map((hedge) => (
          <li key={hedge.hedgeId}>
            <HedgeRow hedge={hedge} bookNames={bookNames} />
          </li>
        ))}
      </ul>
      {remaining > 0 && (
        <button type="button" className={styles.moreBtn} onClick={() => setLimit((n) => n + PAGE)}>
          Show {Math.min(PAGE, remaining)} more ({remaining} hidden)
        </button>
      )}
    </div>
  );
}
