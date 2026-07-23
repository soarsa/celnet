/**
 * AggregatedBookWorkspace — the FI Aggregated Book price view (ADR-0022 D3). A
 * user picks a defined aggregated book and subscribes to its live composite: a
 * dealer-style grid, one row per instrument, showing the bond identity (name +
 * ISIN + CUSIP), the consolidated best bid/offer + firm sizes, a confidence
 * meter, and an expandable per-LP breakdown (each member's own two-way + a stale
 * indicator when it was excluded from the composite). Registered FI-only in the
 * rail, so it appears only under the Fixed Income domain.
 *
 * The composite is a READ line (any authenticated user) — there is NO click-to-
 * trade token on it, so no execute action is offered (honest: executable two-way
 * is the RFS/desk path). The live store (`useAggregatedBook`) opens its own
 * multiplexed session, applies snapshot→update, and tears down on unmount / book
 * change. Prices are clean prices per 100 face (the LP-SIM Treasury convention).
 */

import { useCallback, useEffect, useMemo, useState } from "react";

import { useApp } from "../app/AppContext";
import { Button } from "../components/Button";
import type { AggregatedBookDesc, AggregatedInstrument } from "../data/contract";
import { useAggregatedBook } from "../hooks/useAggregatedBook";
import { fmtClock, fmtCompact } from "../lib/format";
import styles from "./AggregatedBookWorkspace.module.css";

/** A clean price per 100 face to 3 decimals; a dash for an absent (0) composite. */
function fmtPx(x: number): string {
  return x > 0 ? x.toFixed(3) : "—";
}

/** A firm size rendered compactly (e.g. 3_000_000 → "3m"); a dash for none. */
function fmtSize(x: number): string {
  return x > 0 ? fmtCompact(x) : "—";
}

/** A confidence 0..1 rendered as a whole-percent string. */
function fmtConfidence(c: number): string {
  return `${Math.round(Math.max(0, Math.min(1, c)) * 100)}%`;
}

/** One expandable composite row: identity + best two-way + confidence + members. */
function InstrumentRow({
  instrument,
  expanded,
  onToggle,
}: {
  instrument: AggregatedInstrument;
  expanded: boolean;
  onToggle: () => void;
}): React.ReactElement {
  const conf = Math.max(0, Math.min(1, instrument.confidence));
  const memberCount = instrument.contributions.length;
  const freshCount = instrument.contributions.filter((c) => !c.stale).length;
  return (
    <>
      <div
        className={`${styles.row} ${expanded ? styles.rowOpen : ""}`}
        role="row"
        onClick={onToggle}
        aria-expanded={expanded}
        aria-label={`${instrument.displayName || instrument.instrumentId} composite`}
      >
        <span className={styles.discloseCell} role="cell">
          <span className={`${styles.disclose} ${expanded ? styles.discloseOpen : ""}`} aria-hidden="true">
            ▸
          </span>
        </span>
        <span className={styles.identCell} role="cell">
          <span className={styles.identName}>
            {instrument.displayName || instrument.instrumentId}
          </span>
          <span className={styles.identCodes}>
            {instrument.isin && <span className={styles.code}>ISIN {instrument.isin}</span>}
            {instrument.cusip && <span className={styles.code}>CUSIP {instrument.cusip}</span>}
          </span>
        </span>
        <span className={`num ${styles.bidCell}`} role="cell" title="Consolidated best bid (max fresh member bid)">
          {fmtPx(instrument.bestBid)}
        </span>
        <span className={`num ${styles.sizeCell}`} role="cell" title="Firm size at the best bid">
          {fmtSize(instrument.bidSize)}
        </span>
        <span className={`num ${styles.offerCell}`} role="cell" title="Consolidated best offer (min fresh member offer)">
          {fmtPx(instrument.bestOffer)}
        </span>
        <span className={`num ${styles.sizeCell}`} role="cell" title="Firm size at the best offer">
          {fmtSize(instrument.offerSize)}
        </span>
        <span className={styles.confCell} role="cell" title="Composite confidence (coverage · freshness · agreement)">
          <span className={styles.confMeter} aria-hidden="true">
            <span className={styles.confFill} style={{ width: `${conf * 100}%` }} />
          </span>
          <span className={`num ${styles.confPct}`}>{fmtConfidence(conf)}</span>
        </span>
        <span className={styles.contribCell} role="cell" title="Fresh contributors / total members">
          <span className="num">{freshCount}</span>
          <span className={styles.contribSlash}>/</span>
          <span className={`num ${styles.contribTotal}`}>{memberCount}</span>
        </span>
      </div>

      {expanded && (
        <div className={styles.breakdown} role="row">
          <div className={styles.breakdownInner}>
            <div className={styles.breakdownHead}>
              <span className={styles.bkLp}>LP</span>
              <span className={`num ${styles.bkNum}`}>Bid</span>
              <span className={`num ${styles.bkNum}`}>Offer</span>
              <span className={styles.bkState}>State</span>
            </div>
            {instrument.contributions.length === 0 ? (
              <div className={styles.bkEmpty}>No member contributions yet.</div>
            ) : (
              instrument.contributions.map((c) => {
                const setsBid = !c.stale && c.bid === instrument.bestBid && instrument.bestBid > 0;
                const setsOffer = !c.stale && c.offer === instrument.bestOffer && instrument.bestOffer > 0;
                return (
                  <div
                    key={c.lpName}
                    className={`${styles.bkRow} ${c.stale ? styles.bkStale : ""}`}
                    role="row"
                  >
                    <span className={styles.bkLp}>{c.lpName}</span>
                    <span
                      className={`num ${styles.bkNum} ${setsBid ? styles.bkBest : ""}`}
                      title={setsBid ? "sets the composite best bid" : undefined}
                    >
                      {c.bid.toFixed(3)}
                    </span>
                    <span
                      className={`num ${styles.bkNum} ${setsOffer ? styles.bkBest : ""}`}
                      title={setsOffer ? "sets the composite best offer" : undefined}
                    >
                      {c.offer.toFixed(3)}
                    </span>
                    <span className={styles.bkState}>
                      {c.stale ? (
                        <span className={styles.staleTag} title="excluded from the composite (aged out or gated)">
                          stale
                        </span>
                      ) : (
                        <span className={styles.liveTag}>live</span>
                      )}
                    </span>
                  </div>
                );
              })
            )}
          </div>
        </div>
      )}
    </>
  );
}

export function AggregatedBookWorkspace(): React.ReactElement {
  const app = useApp();
  const { auth } = app;

  const [books, setBooks] = useState<AggregatedBookDesc[]>([]);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [expanded, setExpanded] = useState<Set<string>>(new Set());

  const signedIn = auth.user !== undefined && auth.user !== null;

  // Load the roster of defined books (any authenticated user may list them). A
  // deterministic default selection lands on the first ENABLED book.
  useEffect(() => {
    if (!signedIn) {
      setBooks([]);
      setSelectedId(null);
      return;
    }
    let cancelled = false;
    void app.transport
      .listAggregatedBooks()
      .then((list) => {
        if (cancelled) return;
        setBooks(list);
        setLoadError(null);
        setSelectedId((prev) => {
          if (prev && list.some((b) => b.id === prev)) return prev;
          const firstEnabled = list.find((b) => b.enabled) ?? list[0];
          return firstEnabled ? firstEnabled.id : null;
        });
      })
      .catch((e: unknown) => {
        if (cancelled) return;
        setLoadError(e instanceof Error ? e.message : "failed to load aggregated books");
      });
    return () => {
      cancelled = true;
    };
  }, [app.transport, signedIn]);

  const composite = useAggregatedBook(app.transport, signedIn ? selectedId : null);
  const selectedBook = useMemo(
    () => books.find((b) => b.id === selectedId) ?? null,
    [books, selectedId],
  );

  const toggleRow = useCallback((instrumentId: string): void => {
    setExpanded((prev) => {
      const next = new Set(prev);
      if (next.has(instrumentId)) {
        next.delete(instrumentId);
      } else {
        next.add(instrumentId);
      }
      return next;
    });
  }, []);

  // --- the sign-in gate ----------------------------------------------------
  if (!signedIn) {
    return (
      <div className={styles.wrap}>
        <div className={styles.gate}>
          <h2 className={styles.gateTitle}>Aggregated book prices</h2>
          <p className={styles.gateHint}>
            Sign in to view the consolidated composite of an aggregated liquidity book.
          </p>
          <Button variant="primary" onClick={() => app.setSignInOpen(true)}>
            Sign in
          </Button>
        </div>
      </div>
    );
  }

  return (
    <div className={styles.wrap}>
      <div className={styles.head}>
        <div className={styles.headMain}>
          <span className={styles.title}>Aggregated book · live composite</span>
          <span className={styles.note}>
            consolidated best bid/offer across the book&apos;s inbound liquidity members ·
            expand a row for the per-LP breakdown
          </span>
        </div>
        {composite.baselined && selectedBook && (
          <div className={styles.status} aria-live="polite">
            <span className={styles.statusDot} data-live="true" aria-hidden="true" />
            <span className={styles.statusText}>
              seq <span className="num">{composite.sequence.toString()}</span> ·{" "}
              {fmtClock(composite.epochNanos)}
            </span>
          </div>
        )}
      </div>

      {loadError && <p className={styles.banner}>{loadError}</p>}

      {books.length === 0 ? (
        <div className={styles.empty}>
          No aggregated books are defined. An administrator can define one in{" "}
          <strong>Administration → Aggregation</strong>.
        </div>
      ) : (
        <div className={styles.selector} role="group" aria-label="select an aggregated book">
          {books.map((b) => (
            <button
              key={b.id}
              type="button"
              className={`${styles.bookBtn} ${selectedId === b.id ? styles.bookBtnActive : ""}`}
              aria-pressed={selectedId === b.id}
              onClick={() => setSelectedId(b.id)}
              disabled={!b.enabled}
              title={
                b.enabled
                  ? `${b.memberConnectionIds.length} member${b.memberConnectionIds.length === 1 ? "" : "s"}`
                  : "disabled — publishes no composite"
              }
            >
              {b.name}
              {!b.enabled && <span className={styles.bookOff}> (off)</span>}
            </button>
          ))}
        </div>
      )}

      {selectedBook && (
        <section className={styles.gridWrap} aria-label={`${selectedBook.name} composite`}>
          {!composite.baselined ? (
            <div className={styles.empty}>
              {selectedBook.enabled
                ? "Awaiting the first composite snapshot…"
                : "This book is disabled — it publishes no composite."}
            </div>
          ) : composite.instruments.length === 0 ? (
            <div className={styles.empty}>
              No instrument currently meets the book&apos;s quorum. Composites appear as its
              members stream fresh quotes.
            </div>
          ) : (
            <div className={styles.grid} role="table" aria-label="aggregated composite lines">
              <div className={`${styles.row} ${styles.headerRow}`} role="row">
                <span className={styles.discloseCell} role="columnheader" aria-label="expand" />
                <span className={styles.identCell} role="columnheader">
                  Instrument
                </span>
                <span className={`num ${styles.bidCell}`} role="columnheader">
                  Bid
                </span>
                <span className={`num ${styles.sizeCell}`} role="columnheader">
                  Size
                </span>
                <span className={`num ${styles.offerCell}`} role="columnheader">
                  Offer
                </span>
                <span className={`num ${styles.sizeCell}`} role="columnheader">
                  Size
                </span>
                <span className={styles.confCell} role="columnheader">
                  Confidence
                </span>
                <span className={styles.contribCell} role="columnheader" title="fresh / total members">
                  LPs
                </span>
              </div>
              {composite.instruments.map((inst) => (
                <InstrumentRow
                  key={inst.instrumentId}
                  instrument={inst}
                  expanded={expanded.has(inst.instrumentId)}
                  onToggle={() => toggleRow(inst.instrumentId)}
                />
              ))}
            </div>
          )}
        </section>
      )}
    </div>
  );
}
