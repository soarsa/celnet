/**
 * AggregatedBookWorkspace — the FI Aggregated Book price view (ADR-0022 D3). A
 * user picks a defined aggregated book and subscribes to its live composite,
 * rendered as a grid of PRICE TILES — one card per security — each showing the
 * bond identity (name + ISIN + CUSIP), the consolidated best bid/offer + firm
 * sizes, a confidence badge, and an expandable per-LP breakdown (each member's
 * own two-way + a stale indicator when it was excluded from the composite).
 * Registered FI-only in the rail, so it appears only under the Fixed Income
 * domain.
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

/** The bid/offer spread in clean-price points (per 100 face); dash if not two-sided. */
function fmtSpread(bid: number, offer: number): string {
  if (bid > 0 && offer > 0 && offer >= bid) return (offer - bid).toFixed(3);
  return "—";
}

/** Confidence band → a semantic class suffix for the badge colour. */
function confBand(c: number): "hi" | "mid" | "lo" {
  if (c >= 0.75) return "hi";
  if (c >= 0.4) return "mid";
  return "lo";
}

/** One price tile: identity + consolidated two-way + confidence + expandable LPs. */
function InstrumentTile({
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
  const band = confBand(conf);
  const twoSided = instrument.bestBid > 0 && instrument.bestOffer > 0;

  return (
    <article
      className={`${styles.tile} ${expanded ? styles.tileOpen : ""}`}
      aria-label={`${instrument.displayName || instrument.instrumentId} composite`}
    >
      <header className={styles.tileHead}>
        <div className={styles.tileIdent}>
          <span className={styles.identName} title={instrument.displayName || instrument.instrumentId}>
            {instrument.displayName || instrument.instrumentId}
          </span>
          <span className={styles.identCodes}>
            {instrument.isin && <span className={styles.code}>ISIN {instrument.isin}</span>}
            {instrument.cusip && <span className={styles.code}>CUSIP {instrument.cusip}</span>}
          </span>
        </div>
        <span
          className={`${styles.confBadge} ${styles[`conf_${band}`]}`}
          title="Composite confidence (coverage · freshness · agreement)"
        >
          <span className={styles.confDot} aria-hidden="true" />
          <span className="num">{fmtConfidence(conf)}</span>
        </span>
      </header>

      <div className={styles.tileQuote} role="group" aria-label="consolidated two-way">
        <div className={`${styles.side} ${styles.sideBid}`}>
          <span className={styles.sideLabel}>BID</span>
          <span className={`num ${styles.sidePx}`} title="Consolidated best bid (max fresh member bid)">
            {fmtPx(instrument.bestBid)}
          </span>
          <span className={`num ${styles.sideSize}`} title="Firm size at the best bid">
            {fmtSize(instrument.bidSize)}
          </span>
        </div>
        <div className={styles.tileMid} aria-hidden="true">
          <span className={styles.midLabel}>SPREAD</span>
          <span className={`num ${styles.midVal}`}>{fmtSpread(instrument.bestBid, instrument.bestOffer)}</span>
        </div>
        <div className={`${styles.side} ${styles.sideOffer}`}>
          <span className={styles.sideLabel}>OFFER</span>
          <span className={`num ${styles.sidePx}`} title="Consolidated best offer (min fresh member offer)">
            {fmtPx(instrument.bestOffer)}
          </span>
          <span className={`num ${styles.sideSize}`} title="Firm size at the best offer">
            {fmtSize(instrument.offerSize)}
          </span>
        </div>
      </div>

      <footer className={styles.tileFoot}>
        <span className={styles.contribChip} title="Fresh contributors / total members">
          <span className={`${styles.contribPulse} ${twoSided ? styles.contribLive : ""}`} aria-hidden="true" />
          <span className="num">{freshCount}</span>
          <span className={styles.contribSlash}>/</span>
          <span className={`num ${styles.contribTotal}`}>{memberCount}</span>
          <span className={styles.contribWord}>&nbsp;LPs</span>
        </span>
        <button
          type="button"
          className={styles.expandBtn}
          onClick={onToggle}
          aria-expanded={expanded}
          disabled={memberCount === 0}
        >
          {expanded ? "Hide LPs" : "Per-LP"}
          <span className={`${styles.expandCaret} ${expanded ? styles.expandCaretOpen : ""}`} aria-hidden="true">
            ▸
          </span>
        </button>
      </footer>

      {expanded && (
        <div className={styles.breakdown}>
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
                <div key={c.lpName} className={`${styles.bkRow} ${c.stale ? styles.bkStale : ""}`}>
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
      )}
    </article>
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
            one price tile per security · open a tile for the per-LP breakdown
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
            <div className={styles.tileGrid} aria-label="aggregated composite tiles">
              {composite.instruments.map((inst) => (
                <InstrumentTile
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
