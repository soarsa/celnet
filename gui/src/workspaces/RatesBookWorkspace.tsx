/**
 * RatesBookWorkspace — the booked linear-rates book (fixed-income). The desk's
 * standing OIS positions on the right; a Book ticket on the left to add one. This
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
import { Panel } from "../components/Panel";
import { principalForScope } from "../data/riskView";
import { DEFAULT_USD_SOFR_CURVE } from "../data/ratesPricing";
import { fmtRate } from "../lib/format";
import { capabilityDenialTitle } from "../lib/capabilityMatrix";
import type { BookDesc, EntityDesc, OisDirection, RatesPosition } from "../data/contract";
import styles from "./RatesBookWorkspace.module.css";

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
  const pillar = DEFAULT_USD_SOFR_CURVE.pillars.find((p) => p.tenorYears === 5);
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
        setError(err instanceof Error ? err.message : "failed to load rates positions"),
      );
  }, [app.transport, principal]);

  // The workspace stays mounted (the shell display-toggles panes), so besides the
  // initial mount load, re-read the server state whenever this view becomes the
  // ACTIVE one — that picks up any entity/book the admin added (and any position
  // booked) since the last time the view was shown.
  const isActive = app.workspace === "ratesbook";

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
        setError(err instanceof Error ? err.message : "failed to load entity/book registry");
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
      if (t.entityKey !== null && entities.some((e) => e.key === t.entityKey)) return t;
      const firstBook = books.find((b) => b.entityKey === firstEntity.key);
      return { ...t, entityKey: firstEntity.key, bookKey: firstBook ? firstBook.key : null };
    });
  }, [entities, books]);

  const patch = useCallback((p: Partial<BookTicket>) => setTicket((t) => ({ ...t, ...p })), []);

  /** The books belonging to the selected entity (the filtered Book dropdown). */
  const entityBooks = useMemo(
    () => (ticket.entityKey === null ? [] : books.filter((b) => b.entityKey === ticket.entityKey)),
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
  const selectionResolved = ticket.entityKey !== null && ticket.bookKey !== null;
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

  const isOffline = !app.transport.label.startsWith("live");
  const totalMm = positions.reduce((acc, p) => acc + p.instrument.notional, 0) / MM;

  return (
    <div className={styles.wrap}>
      <Panel material="float" className={styles.ticket} title="Book position">
        {registryEmpty ? (
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
                    patch({ bookKey: e.target.value === "" ? null : Number(e.target.value) })
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
                <input
                  className={styles.input}
                  type="number"
                  min={1}
                  step={1}
                  value={ticket.tenorYears}
                  aria-label="tenor in years"
                  onChange={(e) =>
                    patch({ tenorYears: Math.max(1, Math.trunc(Number(e.target.value))) })
                  }
                />
              </Field>
              <Field label="Fixed %">
                <input
                  className={styles.input}
                  type="number"
                  step={0.01}
                  value={ticket.fixedRatePct}
                  aria-label="fixed rate in percent"
                  onChange={(e) => patch({ fixedRatePct: Number(e.target.value) })}
                />
              </Field>
              <Field label="Notional mm">
                <input
                  className={styles.input}
                  type="number"
                  min={1}
                  step={5}
                  value={ticket.notionalMm}
                  aria-label="notional in millions"
                  onChange={(e) => patch({ notionalMm: Math.max(1, Number(e.target.value)) })}
                />
              </Field>
              <Field label="Side">
                <select
                  className={styles.input}
                  value={ticket.direction}
                  aria-label="swap direction"
                  onChange={(e) => patch({ direction: e.target.value as OisDirection })}
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
              <span className={styles.curveTag}>{DEFAULT_USD_SOFR_CURVE.currency}-SOFR</span>
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
          <span className={styles.engine}>{isOffline ? "in-app book" : "live book"}</span>
          <span className={styles.summary}>
            {positions.length} position{positions.length === 1 ? "" : "s"} ·{" "}
            {totalMm.toLocaleString(undefined, { maximumFractionDigits: 0 })}mm notional
          </span>
        </div>
        {positions.length === 0 ? (
          <p className={styles.empty}>The rates book is empty — book a position to populate it.</p>
        ) : (
          <div className={styles.tableWrap}>
            <table className={styles.table}>
              <thead>
                <tr>
                  <th className={styles.num}>Id</th>
                  <th>Entity</th>
                  <th>Book</th>
                  <th>Instrument</th>
                  <th className={styles.num}>Fixed</th>
                  <th className={styles.num}>Notional</th>
                  <th>Side</th>
                </tr>
              </thead>
              <tbody>
                {positions.map((p) => (
                  <tr key={p.positionId.toString()}>
                    <td className={`${styles.num} ${styles.mono} ${styles.idCell}`}>
                      {p.positionId.toString()}
                    </td>
                    <td>{entityName(p.entity)}</td>
                    <td>{bookName(p.book)}</td>
                    <td className={styles.strong}>{p.instrument.tenorYears}y OIS</td>
                    <td className={`${styles.num} ${styles.mono} ${styles.rate}`}>
                      {fmtRate(p.instrument.fixedRate)}
                    </td>
                    <td className={`${styles.num} ${styles.mono}`}>
                      {(p.instrument.notional / MM).toLocaleString(undefined, {
                        maximumFractionDigits: 1,
                      })}
                      mm
                    </td>
                    <td>{directionLabel(p.instrument.direction)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
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
