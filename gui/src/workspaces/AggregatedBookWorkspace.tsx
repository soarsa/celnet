/**
 * AggregatedBookWorkspace — the FI Aggregated Book price view (ADR-0022 D3). A
 * user picks a defined aggregated book and subscribes to its live composite,
 * rendered as a grid of PRICE TILES — one card per security — each showing the
 * bond identity (name + ISIN + CUSIP), the consolidated best bid/offer + firm
 * sizes, a confidence badge, and an expandable per-LP breakdown (each member's
 * own two-way + a stale indicator when it was excluded from the composite).
 * Registered FI-only in the rail (rail-visible on `view·fixed_income` — a trader
 * read), so it appears under the Fixed Income domain. A holder of the granular
 * `manage_liquidity·fixed_income` capability additionally gets a "Manage" mode (a
 * View/Manage toggle) that hosts the aggregated-book definition editor — members,
 * instrument scope, consolidation tuning, and the per-book OUTBOUND TIERING config
 * (widen / skew before publish) — so venue/liquidity ops manage the book + tiering
 * under Fixed Income WITHOUT full Administer. A user without the cap never sees the
 * Manage toggle (docs/PERMISSIONS-GRANULAR-REVIEW.md §4).
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
import { TableSkeleton } from "../components/TableSkeleton";
import type {
  AggregatedBookDesc,
  AggregatedBookSpec,
  AggregatedInstrument,
  BondDef,
  FixConnection,
} from "../data/contract";
import { useAggregatedBook } from "../hooks/useAggregatedBook";
import { useCachedResource } from "../hooks/useCachedResource";
import { useTableUiState } from "../hooks/useTableUiState";
import { useSettings } from "../hooks/useSettings";
import { AggregationPanel } from "./AggregationPanel";
import { useReferenceData } from "../hooks/useReferenceData";
import { indexBondDefs, resolveBondDef } from "../lib/bondTerms";
import {
  assetTypeTabs,
  filterInstrumentsByAssetType,
  filterInstrumentsBySelection,
  securityOptions,
} from "../lib/aggBookSelection";
import { fmtClock, fmtCompact } from "../lib/format";
import { SecurityDetailsPopover } from "./aggbook/SecurityDetailsPopover";
import { SecuritySelectionControl } from "./aggbook/SecuritySelectionControl";
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

/** One price tile: identity + bond terms + consolidated two-way + confidence + expandable LPs. */
function InstrumentTile({
  instrument,
  bond,
  expanded,
  onToggle,
}: {
  instrument: AggregatedInstrument;
  /** The joined reference-data bond terms, or `null` (non-bond / unseeded). */
  bond: BondDef | null;
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
        <div className={styles.footActions}>
          <SecurityDetailsPopover instrument={instrument} bond={bond} />
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
        </div>
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

  const [expanded, setExpanded] = useState<Set<string>>(new Set());
  // "Manage" mode edits the book roster + per-book tiering right here under Fixed
  // Income. The View composite is a trader read (any FI viewer), but the Manage panel
  // gates on the granular `manage_liquidity·fixed_income` capability (venue/liquidity
  // ops, distinct from super-admin — docs/PERMISSIONS-GRANULAR-REVIEW.md §4); admin
  // holds it via grant-all. A user without it never sees the Manage toggle.
  const canManageLiquidity = auth.can("manage_liquidity", "fixed_income");
  const [connections, setConnections] = useState<FixConnection[]>([]);
  const [manageError, setManageError] = useState<string | null>(null);

  const signedIn = auth.user !== undefined && auth.user !== null;

  // The selected book + View/Manage mode are persisted so they SURVIVE a tab switch
  // (the workspace unmounting) and are restored on return.
  const [ui, setUi] = useTableUiState<{
    selectedId: string | null;
    mode: "view" | "manage";
    assetType: string | null;
  }>(
    "fi-agg-book",
    { selectedId: null, mode: "view", assetType: null },
  );
  const selectedId = ui.selectedId;
  const setSelectedId = useCallback((id: string | null) => setUi({ selectedId: id }), [setUi]);
  const assetType = ui.assetType;
  const setAssetType = useCallback((k: string | null) => setUi({ assetType: k }), [setUi]);
  const mode = canManageLiquidity ? ui.mode : "view";
  const setMode = useCallback((m: "view" | "manage") => setUi({ mode: m }), [setUi]);

  // Stale-while-revalidate cache for the defined-book roster (any authenticated user
  // may list them): the roster SURVIVES the workspace unmounting, so returning shows
  // the selector instantly. A mutation revalidates via `reloadBooks` (a background
  // refresh). The live composite for the selected book is a separate PUSH line
  // (`useAggregatedBook`) that re-subscribes on mount.
  const {
    data: booksData,
    isLoading: booksLoading,
    error: booksError,
    refresh: reloadBooks,
  } = useCachedResource<AggregatedBookDesc[]>(
    "aggBooks",
    () => app.transport.listAggregatedBooks(),
    { enabled: signedIn },
  );
  const books = useMemo(() => booksData ?? [], [booksData]);
  const loadError =
    booksError === undefined || booksError === null
      ? null
      : booksError instanceof Error
        ? booksError.message
        : "failed to load aggregated books";

  // Reconcile the selection against the loaded roster: keep a still-present selection,
  // else land deterministically on the first ENABLED book (else the first / none).
  useEffect(() => {
    if (!signedIn || books.length === 0) return;
    if (selectedId && books.some((b) => b.id === selectedId)) return;
    const firstEnabled = books.find((b) => b.enabled) ?? books[0];
    const next = firstEnabled ? firstEnabled.id : null;
    if (next !== selectedId) setSelectedId(next);
  }, [books, signedIn, selectedId, setSelectedId]);

  // The managed FIX-connection registry feeds the Manage editor's member candidates
  // — an admin-only list, so it is fetched ONLY for admins (a non-admin never issues
  // the admin-gated RPC). Failure degrades silently to the free-text member entry.
  useEffect(() => {
    if (!canManageLiquidity) {
      setConnections([]);
      return;
    }
    let cancelled = false;
    void app.transport
      .listFixConnections()
      .then((list) => {
        if (!cancelled) setConnections(list);
      })
      .catch(() => {
        if (!cancelled) setConnections([]);
      });
    return () => {
      cancelled = true;
    };
  }, [app.transport, canManageLiquidity]);

  // Manage-mode mutation runner: surfaces a failure inline (the panel refetches the
  // roster on success via `reloadBooks`, keeping the View selector in sync).
  const runManage = useCallback(async (action: () => Promise<unknown>): Promise<void> => {
    setManageError(null);
    try {
      await action();
    } catch (e: unknown) {
      setManageError(e instanceof Error ? e.message : "action failed");
    }
  }, []);

  const createBook = useCallback(
    async (spec: AggregatedBookSpec): Promise<AggregatedBookDesc> => {
      const created = await app.transport.createAggregatedBook(spec);
      reloadBooks(); // background revalidate the roster (keeps the View selector in sync)
      return created;
    },
    [app.transport, reloadBooks],
  );
  const updateBook = useCallback(
    async (id: string, spec: AggregatedBookSpec): Promise<AggregatedBookDesc> => {
      const updated = await app.transport.updateAggregatedBook(id, spec);
      reloadBooks();
      return updated;
    },
    [app.transport, reloadBooks],
  );
  const deleteBook = useCallback(
    async (id: string): Promise<unknown> => {
      const ok = await app.transport.deleteAggregatedBook(id);
      reloadBooks();
      return ok;
    },
    [app.transport, reloadBooks],
  );

  const composite = useAggregatedBook(app.transport, signedIn ? selectedId : null);
  const selectedBook = useMemo(
    () => books.find((b) => b.id === selectedId) ?? null,
    [books, selectedId],
  );

  // The instrument reference-data registry (any authenticated user may list it),
  // indexed by instrument id + ISIN/CUSIP so each composite line's static bond
  // terms (issuer · coupon · frequency · day-count · maturity) can be surfaced on
  // the tile. The composite wire message carries only identity + prices; the terms
  // are joined here, client-side — no proto/server/codec change. Degrades to no
  // extra terms when a line has no matching bond definition.
  const refData = useReferenceData(app.transport, signedIn);
  const bondIndex = useMemo(
    () => indexBondDefs(refData.instruments),
    [refData.instruments],
  );

  // The per-user "view only what I want" security selection — a client-side
  // preference persisted through the shared settings store (localStorage). It is
  // sourced from the FULL FI reference-data universe (every bond definition), not
  // merely what is currently streaming. An EMPTY selection shows all (never an
  // accidentally-blank book). The picker options + the display filter are pure
  // (lib/aggBookSelection), so they are directly testable.
  const { settings, update } = useSettings();
  const selection = settings.aggBookInstrumentSelection;
  const secOptions = useMemo(
    () => securityOptions(refData.instruments),
    [refData.instruments],
  );
  const setSelection = useCallback(
    (ids: string[]): void => update({ aggBookInstrumentSelection: ids }),
    [update],
  );
  const selectedInstruments = useMemo(
    () =>
      filterInstrumentsBySelection(
        composite.instruments,
        refData.instruments,
        selection,
      ),
    [composite.instruments, refData.instruments, selection],
  );

  // The ASSET-TYPE axis. A book is defined by its inbound liquidity members; a
  // trader reads it by what KIND of risk each line is (`sub_asset_type` off the
  // reference-data taxonomy). Tabs are derived from what is actually streaming,
  // so an empty tab can never be offered. Computed AFTER the security selection
  // so the counts match what the tab would show, not what it could show.
  const assetTabs = useMemo(
    () => assetTypeTabs(selectedInstruments, refData.instruments),
    [selectedInstruments, refData.instruments],
  );

  // Keep the active tab valid as the composite moves: hold the trader's choice
  // while it still has lines, else fall to the first tab. Never leaves a live
  // book rendering blank because the previously-selected type went quiet.
  useEffect(() => {
    if (assetTabs.length === 0) {
      if (assetType !== null) setAssetType(null);
      return;
    }
    if (assetType !== null && assetTabs.some((t) => t.key === assetType)) return;
    setAssetType(assetTabs[0]?.key ?? null);
  }, [assetTabs, assetType, setAssetType]);

  const shownInstruments = useMemo(
    () =>
      filterInstrumentsByAssetType(
        selectedInstruments,
        refData.instruments,
        assetTabs.length === 0 ? null : assetType,
      ),
    [selectedInstruments, refData.instruments, assetTabs.length, assetType],
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
          <span className={styles.title}>
            Aggregated book · {mode === "manage" ? "manage & tiering" : "live composite"}
          </span>
          <span className={styles.note}>
            {mode === "manage"
              ? "define the book's members, instrument scope, consolidation tuning, and outbound price tiering"
              : "consolidated best bid/offer across the book's inbound liquidity members · one price tile per security · open a tile for the per-LP breakdown"}
          </span>
          <span className={styles.note}>
            Live LP-aggregated <strong>prices</strong> (liquidity) — not your positions, and not a
            risk portfolio.
          </span>
        </div>
        <div className={styles.headAside}>
          {canManageLiquidity && (
            <div className={styles.modeToggle} role="group" aria-label="aggregated book mode">
              <button
                type="button"
                className={`${styles.modeBtn} ${mode === "view" ? styles.modeBtnActive : ""}`}
                aria-pressed={mode === "view"}
                onClick={() => setMode("view")}
              >
                View
              </button>
              <button
                type="button"
                className={`${styles.modeBtn} ${mode === "manage" ? styles.modeBtnActive : ""}`}
                aria-pressed={mode === "manage"}
                onClick={() => setMode("manage")}
              >
                Manage
              </button>
            </div>
          )}
          {mode === "view" && selectedBook && (
            <SecuritySelectionControl
              options={secOptions}
              selected={selection}
              onChange={setSelection}
            />
          )}
          {mode === "view" && composite.baselined && selectedBook && (
            <div className={styles.status} aria-live="polite">
              <span className={styles.statusDot} data-live="true" aria-hidden="true" />
              <span className={styles.statusText}>
                seq <span className="num">{composite.sequence.toString()}</span> ·{" "}
                {fmtClock(composite.epochNanos)}
              </span>
            </div>
          )}
        </div>
      </div>

      {canManageLiquidity && mode === "manage" && (
        <section className={styles.managePanel} aria-label="manage aggregated books">
          {manageError && <p className={styles.banner}>{manageError}</p>}
          <AggregationPanel
            books={books}
            connections={connections}
            onCreate={createBook}
            onUpdate={updateBook}
            onDelete={deleteBook}
            run={runManage}
          />
        </section>
      )}

      {mode === "view" && (
        <>
          {loadError && <p className={styles.banner}>{loadError}</p>}

          {booksLoading ? (
            <TableSkeleton rows={3} label="Loading aggregated books…" />
          ) : books.length === 0 ? (
            <div className={styles.empty}>
              No aggregated books are defined.{" "}
              {canManageLiquidity ? (
                <>
                  Switch to <strong>Manage</strong> above to define one — or use{" "}
                  <strong>Administration → Aggregation</strong>.
                </>
              ) : (
                <>
                  An administrator can define one in{" "}
                  <strong>Administration → Aggregation</strong>.
                </>
              )}
            </div>
          ) : (
            <>
              {/*
                The BOOK selector only earns screen space when there is a choice to
                make. With a single defined book the book is context, not a control,
                and the asset-type strip below is the axis a trader actually reads.
              */}
              {books.length > 1 && (
                <div
                  className={styles.selector}
                  role="group"
                  aria-label="select an aggregated book"
                >
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
              {assetTabs.length > 0 && (
                <div className={styles.selector} role="tablist" aria-label="asset type">
                  {assetTabs.map((t) => (
                    <button
                      key={t.key}
                      type="button"
                      role="tab"
                      className={`${styles.bookBtn} ${assetType === t.key ? styles.bookBtnActive : ""}`}
                      aria-selected={assetType === t.key}
                      onClick={() => setAssetType(t.key)}
                      title={`${t.count} live line${t.count === 1 ? "" : "s"}`}
                    >
                      {t.label}
                      <span className={styles.bookOff}> {t.count}</span>
                    </button>
                  ))}
                </div>
              )}
            </>
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
              ) : shownInstruments.length === 0 ? (
                <div className={styles.empty}>
                  None of your selected securities are currently quoting in this book.{" "}
                  <button type="button" className={styles.linkBtn} onClick={() => setSelection([])}>
                    Show all
                  </button>
                </div>
              ) : (
                <div className={styles.tileGrid} aria-label="aggregated composite tiles">
                  {shownInstruments.map((inst) => (
                    <InstrumentTile
                      key={inst.instrumentId}
                      instrument={inst}
                      bond={resolveBondDef(bondIndex, inst)}
                      expanded={expanded.has(inst.instrumentId)}
                      onToggle={() => toggleRow(inst.instrumentId)}
                    />
                  ))}
                </div>
              )}
            </section>
          )}
        </>
      )}
    </div>
  );
}
