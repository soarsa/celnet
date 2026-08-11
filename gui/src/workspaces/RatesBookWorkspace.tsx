/**
 * RatesBookWorkspace — the Positions & Booking LENS of the unified, one-per-book
 * `BookWorkspace` (`fe-fi-migration` #4); the `ratesbook` rail row opens the Book
 * on this lens. Behaviour is unchanged — it is still the booked linear-rates book
 * (fixed-income). The desk's standing OIS positions on the right; a Book ticket on
 * the left to add one. This
 * is the outstanding "rates Book" surface: positions are listed from the server's
 * in-memory rates book and a Book action persists a new one (an accepted desk
 * deal also books a position here, so a fill from the Quoting workspace appears).
 *
 * One contract, two transports (GUI-DESIGN §6.2): the workspace talks ONLY to the
 * `CelnetTransport` rates-Book seam (`bookRatesPosition` / `listRatesPositions`),
 * so the SAME book renders + grows through the deterministic in-app source and the
 * live `RiskService` edge. The Book ticket fields are GENUINE inputs (seeded from
 * the curve pillars), never hardcoded results.
 *
 * Entity / Book are picked by NAME from the admin-managed registry
 * (`listEntities` / `listBooks`, callable by any authenticated trader), and the
 * selection is resolved to the `RatesPosition`'s `(entity, book)` `uint32` keys on
 * submit — the position wire is UNCHANGED, it still carries numeric partition
 * keys. The Book dropdown filters to the selected entity's books, and the
 * positions table resolves each key back to its registry NAME for display (a raw
 * number is never shown; an unknown key falls back to `#<key>`).
 */

import { useCallback, useEffect, useMemo, useState } from "react";
import { useApp } from "../app/AppContext";
import { Button } from "../components/Button";
import { DataTable } from "../components/DataTable";
import { Panel } from "../components/Panel";
import { TableSearch } from "../components/TableSearch";
import { useGridState } from "../hooks/useGridState";
import { useTableFilter } from "../hooks/useTableFilter";
import type { ColumnDef } from "../lib/grid";
import { principalForScope } from "../data/riskView";
import { DEFAULT_USD_SOFR_CURVE } from "../data/ratesPricing";
import { fmtRate, fmtCompact } from "../lib/format";
import { capabilityDenialTitle } from "../lib/capabilityMatrix";
import type {
  BookDesc,
  EntityDesc,
  OisDirection,
  RatesPosition,
} from "../data/contract";
import { pillarYears } from "../data/contract";
import { capabilityAssetForDomain, ratesPositionAsset } from "../data/assetClass";
import styles from "./RatesBookWorkspace.module.css";

import { NumberField } from "../components/NumberField";

const MM = 1_000_000;

/** The Book ticket input model (the editable booking form). */
interface BookTicket {
  /** The selected legal entity's `EntityDesc.key`, or `null` until one is chosen. */
  entityKey: number | null;
  /** The selected book's `BookDesc.key`, or `null` until one is chosen. */
  bookKey: number | null;
  tenorYears: number;
  fixedRatePct: number;
  notionalMm: number;
  direction: OisDirection;
}

function defaultTicket(): BookTicket {
  const pillar = DEFAULT_USD_SOFR_CURVE.pillars.find(
    (p) => pillarYears(p.tenor) === 5,
  );
  const parPct = (pillar?.parRate ?? 0.04) * 100;
  return {
    entityKey: null,
    bookKey: null,
    tenorYears: 5,
    fixedRatePct: Number(parPct.toFixed(4)),
    notionalMm: 50,
    direction: "RECEIVE_FIXED",
  };
}

function directionLabel(direction: OisDirection): string {
  return direction === "PAY_FIXED" ? "Pay" : "Receive";
}

export function RatesBookWorkspace(): React.ReactElement {
  const app = useApp();
  const principal = useMemo(() => principalForScope(app.scope), [app.scope]);
  // Hard asset separation: the Book's Positions & Booking lens shows ONLY the
  // active domain's asset class. The rates book (booking ticket + positions) is
  // fixed-income (OIS); under the FX Options domain the positions table is empty
  // and the rates booking ticket is replaced by an honest note.
  const activeAsset = capabilityAssetForDomain(app.activeDomain);
  const isRatesDomain = activeAsset === "fixed_income";

  const [positions, setPositions] = useState<RatesPosition[]>([]);
  const [entities, setEntities] = useState<EntityDesc[]>([]);
  const [books, setBooks] = useState<BookDesc[]>([]);
  const [ticket, setTicket] = useState<BookTicket>(defaultTicket);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const refresh = useCallback(() => {
    void app.transport
      .listRatesPositions({ ...(principal ? { principal } : {}) })
      .then((res) => {
        setPositions(res.positions);
        setError(null);
      })
      .catch((err) =>
        setError(
          err instanceof Error ? err.message : "failed to load rates positions",
        ),
      );
  }, [app.transport, principal]);

  // fe-fi-migration #6: this panel is now the "positions" LENS BODY of the unified
  // `book` workspace (the standalone `ratesbook` rail row was collapsed away). The
  // shell display-toggles panes and BookWorkspace conditionally mounts this lens,
  // so besides the initial mount load, re-read the server state whenever the Book
  // workspace becomes ACTIVE — that picks up any entity/book the admin added (and
  // any position booked) since the last time the view was shown.
  const isActive = app.workspace === "book";

  useEffect(() => {
    refresh();
  }, [refresh, isActive]);

  // The named entity/book registry that backs the dropdowns + display name
  // resolution. Listing is callable by any authenticated user, so this loads for
  // a trader too (no admin gate). Reload on mount, when the transport changes, and
  // when the view becomes active (so newly-registered entities/books appear
  // without a full reload).
  useEffect(() => {
    let cancelled = false;
    void Promise.all([app.transport.listEntities(), app.transport.listBooks()])
      .then(([nextEntities, nextBooks]) => {
        if (cancelled) return;
        setEntities(nextEntities);
        setBooks(nextBooks);
      })
      .catch((err) => {
        if (cancelled) return;
        setError(
          err instanceof Error
            ? err.message
            : "failed to load entity/book registry",
        );
      });
    return () => {
      cancelled = true;
    };
  }, [app.transport, isActive]);

  // Default the ticket selection to the first entity (and its first book) once
  // the registry loads, so the form is bookable without manual selection.
  useEffect(() => {
    const firstEntity = entities[0];
    if (!firstEntity) return;
    setTicket((t) => {
      if (t.entityKey !== null && entities.some((e) => e.key === t.entityKey))
        return t;
      const firstBook = books.find((b) => b.entityKey === firstEntity.key);
      return {
        ...t,
        entityKey: firstEntity.key,
        bookKey: firstBook ? firstBook.key : null,
      };
    });
  }, [entities, books]);

  const patch = useCallback(
    (p: Partial<BookTicket>) => setTicket((t) => ({ ...t, ...p })),
    [],
  );

  /** The books belonging to the selected entity (the filtered Book dropdown). */
  const entityBooks = useMemo(
    () =>
      ticket.entityKey === null
        ? []
        : books.filter((b) => b.entityKey === ticket.entityKey),
    [books, ticket.entityKey],
  );

  /** Resolve an entity key to its registry name (fallback `#<key>`). */
  const entityName = useCallback(
    (key: number): string => {
      const entity = entities.find((e) => e.key === key);
      return entity ? entity.name : `#${key}`;
    },
    [entities],
  );

  /** Resolve a book key to its registry name (fallback `#<key>`). */
  const bookName = useCallback(
    (key: number): string => {
      const book = books.find((b) => b.key === key);
      return book ? book.name : `#${key}`;
    },
    [books],
  );

  // Selecting an entity narrows the Book dropdown; keep the book selection valid
  // (reset to the entity's first book when the current one no longer belongs).
  const selectEntity = useCallback(
    (entityKey: number) => {
      const owned = books.filter((b) => b.entityKey === entityKey);
      const firstOwned = owned[0];
      setTicket((t) => ({
        ...t,
        entityKey,
        bookKey:
          t.bookKey !== null && owned.some((b) => b.key === t.bookKey)
            ? t.bookKey
            : firstOwned
              ? firstOwned.key
              : null,
      }));
    },
    [books],
  );

  const registryEmpty = entities.length === 0;

  // Capability gating (slice 5): booking a rates position is gated on
  // `book·fixed_income` (disabled + tooltip, never hidden; handler no-ops
  // defensively, the server still enforces). Anonymous ⇒ permissive.
  const canBook = app.auth.can("book", "fixed_income");
  const bookDeniedTitle = capabilityDenialTitle("book", "fixed_income");
  const selectionResolved =
    ticket.entityKey !== null && ticket.bookKey !== null;
  const canSubmit = canBook && selectionResolved && !registryEmpty;

  const book = useCallback(async () => {
    if (!canBook) return;
    // Resolve the NAMED selection to the wire's `uint32` partition keys. The
    // position wire is unchanged — it still carries numeric `entity` / `book`.
    if (ticket.entityKey === null || ticket.bookKey === null) {
      setError("select an entity and a book before booking");
      return;
    }
    const entityKey = ticket.entityKey;
    const bookKey = ticket.bookKey;
    setBusy(true);
    setError(null);
    try {
      await app.transport.bookRatesPosition({
        position: {
          // A placeholder id (0) lets the server mint a stable position id.
          positionId: 0n,
          entity: entityKey,
          book: bookKey,
          instrument: {
            tenorYears: ticket.tenorYears,
            fixedRate: ticket.fixedRatePct / 100,
            notional: ticket.notionalMm * MM,
            direction: ticket.direction,
          },
        },
        ...(principal ? { principal } : {}),
      });
      refresh();
    } catch (err) {
      setError(err instanceof Error ? err.message : "failed to book position");
    } finally {
      setBusy(false);
    }
  }, [app.transport, principal, ticket, refresh, canBook]);

  // Asset-scope the book to the active domain BEFORE the search filter composes
  // on top (rates positions ⇒ shown under Fixed Income, hidden under FX Options).
  const scopedPositions = useMemo(
    () => positions.filter((p) => ratesPositionAsset(p) === activeAsset),
    [positions, activeAsset],
  );

  const { query, setQuery, filtered } = useTableFilter(
    scopedPositions,
    (p) =>
      [
        p.positionId.toString(),
        entityName(p.entity),
        bookName(p.book),
        `${p.instrument.tenorYears}y OIS`,
        fmtRate(p.instrument.fixedRate),
        fmtCompact(p.instrument.notional),
        directionLabel(p.instrument.direction),
      ].join(" "),
  );

  // The column model. `accessor` is the canonical TEXT projection (search,
  // filter, export); `cell` carries the presentation the hand-rolled table had;
  // `sortValue` is the ORDERED projection — without it "1000" would sort before
  // "50" and a 10y swap before a 2y one.
  const columns = useMemo<ReadonlyArray<ColumnDef<RatesPosition>>>(
    () => [
      {
        key: "id",
        header: "Id",
        width: 90,
        align: "right",
        accessor: (p) => p.positionId.toString(),
        // A position id is a bigint on the wire; Number() is exact well past any
        // realistic id, and the text projection above stays the source of truth.
        sortValue: (p) => Number(p.positionId),
        sortKey: "id",
        filter: { kind: "text" },
      },
      {
        key: "entity",
        header: "Entity",
        width: 160,
        align: "left",
        accessor: (p) => entityName(p.entity),
        sortKey: "entity",
        filter: { kind: "select" },
      },
      {
        key: "book",
        header: "Book",
        width: 160,
        align: "left",
        accessor: (p) => bookName(p.book),
        sortKey: "book",
        filter: { kind: "select" },
      },
      {
        key: "instrument",
        header: "Instrument",
        width: 120,
        align: "left",
        accessor: (p) => `${p.instrument.tenorYears}y OIS`,
        cell: (p) => (
          <span className={styles.strong}>{p.instrument.tenorYears}y OIS</span>
        ),
        sortValue: (p) => p.instrument.tenorYears,
        sortKey: "instrument",
        filter: { kind: "select" },
      },
      {
        key: "fixed",
        header: "Fixed",
        width: 100,
        align: "right",
        accessor: (p) => fmtRate(p.instrument.fixedRate),
        cell: (p) => <span className={styles.rate}>{fmtRate(p.instrument.fixedRate)}</span>,
        sortValue: (p) => p.instrument.fixedRate,
        sortKey: "fixed",
        filter: { kind: "range" },
      },
      {
        key: "notional",
        header: "Notional",
        width: 110,
        align: "right",
        accessor: (p) => fmtCompact(p.instrument.notional),
        sortValue: (p) => p.instrument.notional,
        sortKey: "notional",
        filter: { kind: "range" },
      },
      {
        key: "side",
        header: "Side",
        width: 90,
        align: "left",
        accessor: (p) => directionLabel(p.instrument.direction),
        sortKey: "side",
        filter: { kind: "select" },
      },
    ],
    [entityName, bookName],
  );

  const grid = useGridState<RatesPosition>({
    tableId: "fi-rates-book",
    columns,
    rows: filtered,
    allRows: scopedPositions,
  });

  const isOffline = !app.transport.label.startsWith("live");
  const totalNotional = scopedPositions.reduce((acc, p) => acc + p.instrument.notional, 0);

  return (
    <div className={styles.wrap}>
      <Panel material="float" className={styles.ticket} title="Book position">
        {!isRatesDomain ? (
          <p className={styles.empty}>
            Booking here is the fixed-income (OIS) rates book. Switch to the Fixed
            Income domain to book and view rates positions.
          </p>
        ) : registryEmpty ? (
          <p className={styles.empty}>
            No legal entities are registered yet. Add entities and books in
            Administration before booking a position.
          </p>
        ) : (
          <>
            <div className={styles.ticketGrid}>
              <Field label="Entity">
                <select
                  className={styles.input}
                  value={ticket.entityKey ?? ""}
                  aria-label="legal entity"
                  onChange={(e) => selectEntity(Number(e.target.value))}
                >
                  {entities.map((entity) => (
                    <option key={entity.key} value={entity.key}>
                      {entity.name}
                    </option>
                  ))}
                </select>
              </Field>
              <Field label="Book">
                <select
                  className={styles.input}
                  value={ticket.bookKey ?? ""}
                  aria-label="netting book"
                  disabled={entityBooks.length === 0}
                  onChange={(e) =>
                    patch({
                      bookKey:
                        e.target.value === "" ? null : Number(e.target.value),
                    })
                  }
                >
                  {entityBooks.length === 0 ? (
                    <option value="">No books for this entity</option>
                  ) : (
                    entityBooks.map((b) => (
                      <option key={b.key} value={b.key}>
                        {b.name}
                      </option>
                    ))
                  )}
                </select>
              </Field>
              <Field label="Tenor (y)">
                <NumberField
                  className={styles.input}
                  min={1}
                  step={1}
                  value={ticket.tenorYears}
                  aria-label="tenor in years"
                  onChange={(e) =>
                    patch({
                      tenorYears: Math.max(
                        1,
                        Math.trunc(Number(e.target.value)),
                      ),
                    })
                  }
                />
              </Field>
              <Field label="Fixed %">
                <NumberField
                  className={styles.input}
                  step={0.01}
                  value={ticket.fixedRatePct}
                  aria-label="fixed rate in percent"
                  onChange={(e) =>
                    patch({ fixedRatePct: Number(e.target.value) })
                  }
                />
              </Field>
              <Field label="Notional mm">
                <NumberField
                  className={styles.input}
                  min={1}
                  step={5}
                  value={ticket.notionalMm}
                  aria-label="notional in millions"
                  onChange={(e) =>
                    patch({ notionalMm: Math.max(1, Number(e.target.value)) })
                  }
                />
              </Field>
              <Field label="Side">
                <select
                  className={styles.input}
                  value={ticket.direction}
                  aria-label="swap direction"
                  onChange={(e) =>
                    patch({ direction: e.target.value as OisDirection })
                  }
                >
                  <option value="RECEIVE_FIXED">Receive</option>
                  <option value="PAY_FIXED">Pay</option>
                </select>
              </Field>
            </div>
            <div className={styles.ticketFoot}>
              <Button
                variant="primary"
                disabled={busy || !canSubmit}
                onClick={book}
                title={canBook ? undefined : bookDeniedTitle}
              >
                Book position
              </Button>
              <span className={styles.curveTag}>
                {DEFAULT_USD_SOFR_CURVE.currency}-SOFR
              </span>
            </div>
          </>
        )}
        {error && (
          <p className={styles.error} role="alert">
            {error}
          </p>
        )}
      </Panel>

      <Panel className={styles.positions} title="Rates book">
        <div className={styles.head}>
          <span className={styles.engine}>
            {isOffline ? "in-app book" : "live book"}
          </span>
          <span className={styles.summary}>
            {scopedPositions.length} position{scopedPositions.length === 1 ? "" : "s"} ·{" "}
            {fmtCompact(totalNotional)} notional
          </span>
        </div>
        {scopedPositions.length === 0 ? (
          <p className={styles.empty}>
            {isRatesDomain
              ? "The rates book is empty — book a position to populate it."
              : "No FX-option positions in this book. The rates book is fixed-income (OIS) — switch to the Fixed Income domain to book and view positions; the FX options book's risk is under the Aggregate Risk lens."}
          </p>
        ) : (
          <>
            {/* "N of M" reports the survivors of BOTH stages of the pipeline —
                the global search AND the per-column filters — so the count never
                claims rows that a column filter has since removed. */}
            <TableSearch
              query={query}
              onQueryChange={setQuery}
              shown={grid.shown}
              total={grid.total}
              label="Search positions"
              placeholder="Filter positions…"
            />
            <DataTable
              label="Rates book positions"
              columns={columns}
              grid={grid}
              rowKey={(p) => p.positionId.toString()}
              hideRowCount
              emptyState={
                query.trim() === ""
                  ? "No positions match the current column filters."
                  : `No positions match “${query}”.`
              }
            />
          </>
        )}
      </Panel>
    </div>
  );
}

function Field({
  label,
  children,
}: {
  label: string;
  children: React.ReactNode;
}): React.ReactElement {
  return (
    <label className={styles.field}>
      <span className={styles.fieldLabel}>{label}</span>
      {children}
    </label>
  );
}
