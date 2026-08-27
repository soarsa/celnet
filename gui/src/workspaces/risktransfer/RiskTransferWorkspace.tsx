/**
 * RiskTransferWorkspace — the FI Risk Transfer TICKET (docs/RISK-TRANSFER-
 * REQUIREMENTS.md §9.1). The manual move of EXISTING risk between risk portfolios —
 * the complement to routing (which auto-assigns NEW fills). The trader:
 *
 *   1. picks a SOURCE portfolio, then multi-selects its positions — listed from the
 *      server (`listPositions` + `listRatesPositions`, both filtered on the routing
 *      graph's risk-book stamp), each showing its own server-computed risk;
 *   2. picks a TARGET portfolio (and, optionally, hands off to a named trader) — the
 *      ticket INFERS the kind (same desk ⇒ re-attribution; other desk ⇒ desk-to-desk;
 *      a trader ⇒ hand-off) and explains what it means;
 *   3. chooses Full or Partial quantity (a notional bounded by the selection);
 *   4. leaves the price at mid / mark (read-only, at par) or toggles an AGREED override
 *      that requires a reason;
 *   5. reads a before/after PREVIEW (source risk ↓, target risk ↑, realised P&L in the
 *      source at the transfer price), then submits.
 *
 * Submit calls `initiateRiskTransfer`: a re-attribution returns BOOKED (applied
 * immediately); a desk-to-desk / trader hand-off returns PENDING for the counterparty's
 * inbox. On success the ticket re-fetches `listRiskBookRisk` and shows the move landed.
 *
 * The position lots were once SYNTHESISED here — a book's aggregate risk sliced into
 * lots with hashed ids — which made the screen look complete while every transfer was
 * refused `position <id> is not booked in either book`. They are now the server's own
 * positions; the moved economics are driven by the quantity against them (see
 * transferModel.ts). Reads the transport + auth via `useApp()`.
 */

import { useCallback, useEffect, useMemo, useState } from "react";

import { useApp } from "../../app/AppContext";
import type {
  DeskDesc,
  RiskBook,
  RiskBookRisk,
  RiskTransfer,
  RiskVector,
  TransferPriceBasis,
  UserDesc,
} from "../../data/contract";
import { fmtCompact, fmtSigned } from "../../lib/format";
import {
  computePreview,
  effectiveDeskId,
  inferKind,
  kindMeaning,
  PAR_MARK,
  type PositionLine,
} from "./transferModel";
import { RiskTransferInboxWorkspace } from "./RiskTransferInboxWorkspace";
import { RiskTransferAuditWorkspace } from "./RiskTransferAuditWorkspace";
import styles from "./RiskTransferWorkspace.module.css";

import { MagnitudeField } from "../../components/MagnitudeField";
import { NumberField } from "../../components/NumberField";

/** The tab the consolidated Risk Transfer surface shows: the initiate ticket
 * (default), the accept/reject inbox, or the immutable audit trail. `ticket` is the
 * default; `inbox` / `audit` are the deep-link targets for the retired
 * "Transfer Inbox" / "Transfer Audit" rail entries. */
export type RiskTransferTab = "ticket" | "inbox" | "audit";

/** One row per tab the consolidated surface spans: its id, toggle label, and whether
 * it is a write-class (initiate / accept) tab gated on `risk_transfer` — the audit
 * trail sits on the `view·FI` floor, so it shows for any FI trader. */
const RISK_TRANSFER_TABS: readonly {
  tab: RiskTransferTab;
  label: string;
  needsTransfer: boolean;
}[] = [
  { tab: "ticket", label: "Risk Transfer", needsTransfer: true },
  { tab: "inbox", label: "Inbox", needsTransfer: true },
  { tab: "audit", label: "Audit", needsTransfer: false },
];

/**
 * RiskTransferWorkspace — the tabbed shell composing the {@link
 * RiskTransferTicketPanel} initiate ticket, the {@link RiskTransferInboxWorkspace}
 * accept/reject inbox, and the {@link RiskTransferAuditWorkspace} audit trail as
 * sibling tabs (formerly three separate rail rows). Reuses the Risk Dashboard /
 * Pricing "Book → Risk" tab primitive VERBATIM: a slim segmented bar above the active
 * panel, which fills the remaining pane height and scrolls its OWN content (the Shell
 * pane is overflow:hidden with a definite height). Only the active tab's body mounts,
 * so each panel's effects (the ticket's roster load, the inbox subscription, the audit
 * query) fire only while it is on screen.
 *
 * Each tab keeps its ORIGINAL capability gate: the initiate + inbox tabs are hidden
 * from a trader lacking `risk_transfer·fixed_income` (a booking-only FI trader), who
 * still reaches the surface for the `view`-floor audit trail — the active tab clamps
 * to the first VISIBLE tab so a hidden tab is never shown empty. `can` is permissive
 * signed-out, so pre-login all three render.
 */
export function RiskTransferWorkspace({
  initialTab = "ticket",
}: {
  /** The initial tab — the retired `transferinbox` deep-link opens on `inbox`, the
   * `transferaudit` deep-link on `audit`; the rail's Risk Transfer entry (and stories
   * / tests) default to `ticket`. */
  initialTab?: RiskTransferTab;
} = {}): React.ReactElement {
  const { auth } = useApp();
  // The initiate + inbox tabs are write-class (initiate / accept), gated on the narrow
  // `risk_transfer` capability the server enforces; the audit trail stays `view`.
  const canTransfer = auth.can("risk_transfer", "fixed_income");
  const visibleTabs = RISK_TRANSFER_TABS.filter((t) => !t.needsTransfer || canTransfer);

  const [tab, setTab] = useState<RiskTransferTab>(initialTab);
  // Clamp to a VISIBLE tab so a deep-link (or default) landing on a tab this identity
  // cannot view falls to the first tab it can (the audit trail), never an empty pane.
  const activeTab: RiskTransferTab = visibleTabs.some((t) => t.tab === tab)
    ? tab
    : (visibleTabs[0]?.tab ?? "audit");

  return (
    <div className={styles.shell}>
      <div className={styles.tabBar} role="group" aria-label="risk transfer view">
        {visibleTabs.map((t) => (
          <button
            key={t.tab}
            type="button"
            className={`${styles.tabBtn} ${activeTab === t.tab ? styles.tabBtnActive : ""}`}
            aria-pressed={activeTab === t.tab}
            data-testid={`risk-transfer-tab-${t.tab}`}
            onClick={() => setTab(t.tab)}
          >
            {t.label}
          </button>
        ))}
      </div>
      <div className={styles.tabPanel}>
        {activeTab === "ticket" ? (
          <RiskTransferTicketPanel />
        ) : activeTab === "inbox" ? (
          <RiskTransferInboxWorkspace />
        ) : (
          <RiskTransferAuditWorkspace />
        )}
      </div>
    </div>
  );
}

/** A desk's display name from its id, falling back to the id itself. */
function deskName(id: string, desks: readonly DeskDesc[]): string {
  if (id.length === 0) return "—";
  return desks.find((d) => d.id === id)?.name ?? id;
}

/**
 * The risk a position carries when the server sent no vector at all — distinct from a
 * position the server priced at zero, but indistinguishable to the arithmetic below, so
 * it is named once here rather than spelled out at each call site.
 */
const ZERO_RISK: RiskVector = { dv01: 0, delta: 0, gamma: 0, vega: 0, theta: 0 };

/**
 * RiskTransferTicketPanel — the FI Risk Transfer INITIATE ticket (this file's original
 * body, extracted VERBATIM as the default "Risk Transfer" tab of the consolidated
 * {@link RiskTransferWorkspace} shell). See the file header for the full ticket flow.
 */
function RiskTransferTicketPanel(): React.ReactElement {
  const app = useApp();
  const { auth } = app;
  const signedIn = auth.user !== null && auth.user !== undefined;

  const [books, setBooks] = useState<RiskBook[]>([]);
  const [risk, setRisk] = useState<RiskBookRisk[]>([]);
  const [desks, setDesks] = useState<DeskDesc[]>([]);
  const [traders, setTraders] = useState<UserDesc[]>([]);
  const [loadError, setLoadError] = useState<string | null>(null);

  // Ticket inputs.
  const [sourceBookId, setSourceBookId] = useState("");
  const [selectedIds, setSelectedIds] = useState<ReadonlySet<string>>(new Set());
  const [targetBookId, setTargetBookId] = useState("");
  const [targetTrader, setTargetTrader] = useState("");
  const [quantityFull, setQuantityFull] = useState(true);
  // Held as a parsed number: MagnitudeField refuses to commit anything that is
  // not one, so this can never hold a half-typed or nonsense quantity.
  const [partialInput, setPartialInput] = useState<number | null>(null);
  const [basis, setBasis] = useState<TransferPriceBasis>("MID");
  const [agreedInput, setAgreedInput] = useState(String(PAR_MARK));
  const [reason, setReason] = useState("");

  // Submit lifecycle.
  const [submitting, setSubmitting] = useState(false);
  const [submitError, setSubmitError] = useState<string | null>(null);
  const [result, setResult] = useState<RiskTransfer | null>(null);
  const [afterMove, setAfterMove] = useState<{ source: RiskBookRisk | null; target: RiskBookRisk | null } | null>(null);

  const loadRisk = useCallback(
    async (transport: typeof app.transport): Promise<RiskBookRisk[]> => {
      const rows = await transport.listRiskBookRisk();
      setRisk(rows);
      return rows;
    },
    [],
  );

  useEffect(() => {
    if (!signedIn) {
      setBooks([]);
      setRisk([]);
      setDesks([]);
      setTraders([]);
      return;
    }
    let cancelled = false;
    setLoadError(null);
    void (async (): Promise<void> => {
      try {
        const [b, r, d] = await Promise.all([
          app.transport.listRiskBooks(),
          app.transport.listRiskBookRisk(),
          app.transport.listDesks(),
        ]);
        if (cancelled) return;
        setBooks(b);
        setRisk(r);
        setDesks(d);
      } catch (e: unknown) {
        if (!cancelled) setLoadError(e instanceof Error ? e.message : "failed to load transfer data");
      }
      // The trader roster is admin-only server-side — load it best-effort so the
      // hand-off picker appears for an admin and is simply absent otherwise.
      try {
        const users = await app.transport.listUsers();
        if (!cancelled) setTraders(users.filter((u) => !u.disabled));
      } catch {
        if (!cancelled) setTraders([]);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [app.transport, signedIn]);

  const enabledBooks = useMemo(() => books.filter((b) => b.enabled), [books]);
  const sourceRow = useMemo(
    () => risk.find((r) => r.bookId === sourceBookId) ?? null,
    [risk, sourceBookId],
  );
  // The source book's REAL positions, from the server.
  //
  // These were previously SYNTHESISED: the book's aggregate risk sliced into lots with
  // ids minted from a hash. The screen looked complete and could never transfer anything —
  // the server refused every one with `position <id> is not booked in either book`,
  // correctly, because those ids named nothing. `RatesPosition` now carries its risk-book
  // stamp and its server-computed DV01, so the ticket can list what is actually there.
  const [lines, setLines] = useState<PositionLine[]>([]);
  const [linesError, setLinesError] = useState<string | null>(null);
  useEffect(() => {
    if (!signedIn || sourceBookId === "") {
      setLines([]);
      return;
    }
    let cancelled = false;
    setLinesError(null);
    void (async (): Promise<void> => {
      try {
        // BOTH asset books, one path. The server transfers FX and rates positions
        // through the same applier (`TransferApplier::classify` probes both books), and
        // the source dropdown offers every enabled risk book — so listing only one asset
        // would leave the ticket dead on the other. Both messages now carry the same
        // `riskBook` stamp and the same `risk` vector, so the two listings differ only in
        // where the notional and the label come from.
        const [fx, rates] = await Promise.all([
          app.transport.listPositions({}),
          app.transport.listRatesPositions({}),
        ]);
        if (cancelled) return;
        const fxLines: PositionLine[] = fx.positions
          .filter((p) => p.riskBook === sourceBookId)
          .map((p) => ({
            id: p.positionId,
            label: `#${p.positionId} · ${p.org.ccyPair.base}${p.org.ccyPair.quote} ${p.optionType}`,
            notionalBase: p.notionalBase,
            risk: p.risk ?? ZERO_RISK,
          }));
        const ratesLines: PositionLine[] = rates.positions
          .filter((p) => p.riskBook === sourceBookId)
          .map((p) => ({
            id: p.positionId,
            label: `#${p.positionId} · ${p.instrument.tenorYears}Y OIS`,
            notionalBase: p.netNotional ?? 0,
            risk: p.risk ?? ZERO_RISK,
          }));
        setLines([...fxLines, ...ratesLines]);
      } catch (e: unknown) {
        if (!cancelled) {
          setLines([]);
          setLinesError(e instanceof Error ? e.message : "failed to load positions");
        }
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [app.transport, signedIn, sourceBookId]);
  const selectedLines = useMemo(
    () => lines.filter((l) => selectedIds.has(l.id.toString())),
    [lines, selectedIds],
  );

  const sourceDesk = useMemo(
    () => (sourceBookId ? effectiveDeskId(sourceBookId, books) : ""),
    [sourceBookId, books],
  );
  const targetDesk = useMemo(
    () => (targetBookId ? effectiveDeskId(targetBookId, books) : ""),
    [targetBookId, books],
  );
  const kind = useMemo(
    () => inferKind(sourceDesk, targetDesk, targetTrader),
    [sourceDesk, targetDesk, targetTrader],
  );
  const targetRow = useMemo(
    () => risk.find((r) => r.bookId === targetBookId) ?? null,
    [risk, targetBookId],
  );

  const selectedTotal = useMemo(
    () => selectedLines.reduce((s, l) => s + l.notionalBase, 0),
    [selectedLines],
  );

  const partialNotional = quantityFull ? null : partialInput;
  const agreedPrice = basis === "AGREED" ? Number(agreedInput) : null;

  const preview = useMemo(
    () =>
      computePreview({
        selected: selectedLines,
        quantityFull,
        partialNotional,
        agreedPrice,
        isAgreed: basis === "AGREED",
        sourceNet: sourceRow?.netNotional ?? 0,
        targetNet: targetRow?.netNotional ?? 0,
      }),
    [selectedLines, quantityFull, partialNotional, agreedPrice, basis, sourceRow, targetRow],
  );

  // Reset the selection + downstream ticket state when the source changes.
  const onSourceChange = useCallback((id: string): void => {
    setSourceBookId(id);
    setSelectedIds(new Set());
    setResult(null);
    setAfterMove(null);
    setSubmitError(null);
  }, []);

  const toggleLine = useCallback((id: string): void => {
    setSelectedIds((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }, []);

  const selectAllLines = useCallback((): void => {
    setSelectedIds(new Set(lines.map((l) => l.id.toString())));
  }, [lines]);

  // The validation gate the submit button + form share (also enforced server-side).
  const partialValid =
    quantityFull ||
    (partialInput !== null && partialInput > 0 && partialInput <= Math.abs(selectedTotal) + 1e-6);
  const agreedValid =
    basis !== "AGREED" || (agreedInput.trim().length > 0 && Number.isFinite(Number(agreedInput)) &&
      reason.trim().length > 0);
  const canSubmit =
    !submitting &&
    sourceBookId.length > 0 &&
    targetBookId.length > 0 &&
    sourceBookId !== targetBookId &&
    selectedLines.length > 0 &&
    partialValid &&
    agreedValid;

  const onSubmit = useCallback(
    async (e: React.FormEvent): Promise<void> => {
      e.preventDefault();
      if (!canSubmit) return;
      setSubmitting(true);
      setSubmitError(null);
      setResult(null);
      setAfterMove(null);
      try {
        const transfer = await app.transport.initiateRiskTransfer({
          kind,
          source: {
            riskBookId: sourceBookId,
            deskId: sourceDesk,
            trader: auth.user?.email ?? "",
            positionIds: selectedLines.map((l) => l.id),
          },
          target: {
            riskBookId: targetBookId,
            deskId: targetDesk,
            trader: targetTrader,
            positionIds: [],
          },
          quantityFull,
          partialNotional: quantityFull ? null : partialInput,
          priceBasis: basis,
          agreedPrice: basis === "AGREED" ? Number(agreedInput) : null,
          reason,
        });
        setResult(transfer);
        const rows = await loadRisk(app.transport);
        setAfterMove({
          source: rows.find((r) => r.bookId === sourceBookId) ?? null,
          target: rows.find((r) => r.bookId === targetBookId) ?? null,
        });
      } catch (err: unknown) {
        setSubmitError(err instanceof Error ? err.message : "the transfer was rejected");
      } finally {
        setSubmitting(false);
      }
    },
    [
      canSubmit, app.transport, kind, sourceBookId, sourceDesk, auth.user, selectedLines,
      targetBookId, targetDesk, targetTrader, quantityFull, partialInput, basis, agreedInput,
      reason, loadRisk,
    ],
  );

  if (!signedIn) {
    return (
      <div className={styles.wrap}>
        <p className={styles.empty}>Sign in to move risk between portfolios.</p>
      </div>
    );
  }

  return (
    <div className={styles.wrap}>
      <header className={styles.head}>
        <h1 className={styles.title}>Risk Transfer</h1>
        <p className={styles.note}>
          Move existing risk between portfolios — the manual complement to routing. A
          re-attribution within one desk books immediately; a cross to another desk or a
          hand-off to another trader lands <strong>Pending</strong> for the counterparty&apos;s
          inbox.
        </p>
      </header>

      {loadError && (
        <p className={styles.error} role="alert">
          {loadError}
        </p>
      )}

      {/* Reported separately from `loadError`: failing to load the source book's
          positions is a different problem from failing to load the books themselves, and
          says something the operator can act on — the ticket has nothing to move. */}
      {linesError !== null && (
        <p className={styles.error} role="alert" data-testid="transfer-positions-error">
          Could not load the source portfolio's positions: {linesError}
        </p>
      )}

      <form className={styles.grid} onSubmit={onSubmit} aria-label="Risk transfer ticket">
        {/* --- 1 · source portfolio + positions --------------------------------- */}
        <section className={styles.card} aria-labelledby="xfer-source-h">
          <h2 className={styles.cardTitle} id="xfer-source-h">
            <span className={styles.step}>1</span> Source &amp; positions
          </h2>
          <label className={styles.field}>
            <span className={styles.fieldLabel}>Source portfolio</span>
            <select
              className={styles.select}
              value={sourceBookId}
              onChange={(e) => onSourceChange(e.target.value)}
              data-testid="xfer-source"
            >
              <option value="">Select a portfolio…</option>
              {enabledBooks.map((b) => (
                <option key={b.id} value={b.id}>
                  {b.name} · desk {deskName(effectiveDeskId(b.id, books), desks)}
                </option>
              ))}
            </select>
          </label>

          {sourceRow && (
            <div className={styles.lines} role="group" aria-label="Positions to move">
              <div className={styles.linesHead}>
                <span className={styles.linesTitle}>Positions ({lines.length})</span>
                <button
                  type="button"
                  className={styles.ghostBtn}
                  onClick={selectAllLines}
                  data-testid="xfer-select-all"
                >
                  Select all
                </button>
              </div>
              {lines.map((l) => {
                const id = l.id.toString();
                const checked = selectedIds.has(id);
                return (
                  <label key={id} className={`${styles.line} ${checked ? styles.lineOn : ""}`}>
                    <input
                      type="checkbox"
                      checked={checked}
                      onChange={() => toggleLine(id)}
                      data-testid={`xfer-line-${id}`}
                    />
                    <span className={styles.lineLabel}>{l.label}</span>
                    <span className={styles.lineRisk}>
                      <span className={styles.mono}>{fmtCompact(l.notionalBase)}</span>
                      <span className={styles.lineGreeks}>
                        {sourceRow.dv01 !== null
                          ? `DV01 ${fmtSigned(l.risk.dv01, 0)}`
                          : `Δ ${fmtSigned(l.risk.delta, 0)} · V ${fmtSigned(l.risk.vega, 0)}`}
                      </span>
                    </span>
                  </label>
                );
              })}
              <p className={styles.hint}>
                Each line is a position the server holds in this portfolio, with its
                server-computed DV01. Selecting one names it by its own position id — the
                identity the transfer is booked against.
              </p>
            </div>
          )}
        </section>

        {/* --- 2 · target + inferred kind --------------------------------------- */}
        <section className={styles.card} aria-labelledby="xfer-target-h">
          <h2 className={styles.cardTitle} id="xfer-target-h">
            <span className={styles.step}>2</span> Target
          </h2>
          <label className={styles.field}>
            <span className={styles.fieldLabel}>Target portfolio</span>
            <select
              className={styles.select}
              value={targetBookId}
              onChange={(e) => setTargetBookId(e.target.value)}
              data-testid="xfer-target"
            >
              <option value="">Select a portfolio…</option>
              {enabledBooks
                .filter((b) => b.id !== sourceBookId)
                .map((b) => (
                  <option key={b.id} value={b.id}>
                    {b.name} · desk {deskName(effectiveDeskId(b.id, books), desks)}
                  </option>
                ))}
            </select>
          </label>

          {traders.length > 0 && (
            <label className={styles.field}>
              <span className={styles.fieldLabel}>Hand off to trader (optional)</span>
              <select
                className={styles.select}
                value={targetTrader}
                onChange={(e) => setTargetTrader(e.target.value)}
                data-testid="xfer-trader"
              >
                <option value="">— none (desk-level) —</option>
                {traders.map((u) => (
                  <option key={u.id} value={u.email}>
                    {u.displayName} · {u.email}
                  </option>
                ))}
              </select>
            </label>
          )}

          {targetBookId.length > 0 && (
            <div className={styles.kind} data-testid="xfer-kind">
              <span className={`${styles.kindBadge} ${styles[`kind_${kind}`]}`}>
                {kind.replace(/_/g, " ").toLowerCase()}
              </span>
              <p className={styles.kindMeaning}>{kindMeaning(kind)}</p>
            </div>
          )}
        </section>

        {/* --- 3 · quantity ----------------------------------------------------- */}
        <section className={styles.card} aria-labelledby="xfer-qty-h">
          <h2 className={styles.cardTitle} id="xfer-qty-h">
            <span className={styles.step}>3</span> Quantity
          </h2>
          <fieldset className={styles.radios}>
            <legend className={styles.srOnly}>Transfer quantity</legend>
            <label className={styles.radio}>
              <input
                type="radio"
                name="qty"
                checked={quantityFull}
                onChange={() => setQuantityFull(true)}
                data-testid="xfer-qty-full"
              />
              <span>Full — move the whole selection</span>
            </label>
            <label className={styles.radio}>
              <input
                type="radio"
                name="qty"
                checked={!quantityFull}
                onChange={() => setQuantityFull(false)}
                data-testid="xfer-qty-partial"
              />
              <span>Partial notional</span>
            </label>
          </fieldset>
          {!quantityFull && (
            <label className={styles.field}>
              <span className={styles.fieldLabel}>
                Notional to move (max {fmtCompact(Math.abs(selectedTotal))})
              </span>
              <MagnitudeField
                className={styles.input}
                min={0}
                max={Math.abs(selectedTotal)}
                value={partialInput}
                onCommit={setPartialInput}
                data-testid="xfer-partial"
              />
            </label>
          )}
        </section>

        {/* --- 4 · transfer price ---------------------------------------------- */}
        <section className={styles.card} aria-labelledby="xfer-price-h">
          <h2 className={styles.cardTitle} id="xfer-price-h">
            <span className={styles.step}>4</span> Transfer price
          </h2>
          <fieldset className={styles.radios}>
            <legend className={styles.srOnly}>Transfer-price basis</legend>
            {(["MID", "MARK_TO_MARKET", "AGREED"] as const).map((b) => (
              <label key={b} className={styles.radio}>
                <input
                  type="radio"
                  name="basis"
                  checked={basis === b}
                  onChange={() => setBasis(b)}
                  data-testid={`xfer-basis-${b}`}
                />
                <span>{b === "MARK_TO_MARKET" ? "Mark-to-market" : b === "MID" ? "Mid" : "Agreed override"}</span>
              </label>
            ))}
          </fieldset>
          {basis !== "AGREED" ? (
            <p className={styles.priceRO} data-testid="xfer-price-readonly">
              Auto-filled at the live composite mark ·{" "}
              <span className={styles.mono}>{PAR_MARK.toFixed(2)}</span> (read-only)
            </p>
          ) : (
            <>
              <label className={styles.field}>
                <span className={styles.fieldLabel}>Agreed price</span>
                <NumberField
                  className={styles.input}
                  step="0.01"
                  value={agreedInput}
                  onChange={(e) => setAgreedInput(e.target.value)}
                  data-testid="xfer-agreed"
                />
              </label>
              <label className={styles.field}>
                <span className={styles.fieldLabel}>Reason (required for an agreed price)</span>
                <textarea
                  className={styles.textarea}
                  value={reason}
                  onChange={(e) => setReason(e.target.value)}
                  rows={2}
                  data-testid="xfer-reason"
                  aria-invalid={basis === "AGREED" && reason.trim().length === 0}
                  placeholder="Why is this crossing off-mark?"
                />
              </label>
            </>
          )}
        </section>

        {/* --- preview + submit ------------------------------------------------- */}
        <section className={`${styles.card} ${styles.previewCard}`} aria-labelledby="xfer-preview-h">
          <h2 className={styles.cardTitle} id="xfer-preview-h">
            Preview
          </h2>
          {selectedLines.length === 0 ? (
            <p className={styles.hint}>Select at least one position to preview the move.</p>
          ) : (
            <div className={styles.preview} data-testid="xfer-preview">
              <div className={styles.previewMove}>
                <Stat label="Notional moved" value={fmtCompact(preview.movedNotional)} />
                <Stat
                  label="Risk moved"
                  value={
                    sourceRow?.dv01 !== null && sourceRow?.dv01 !== undefined
                      ? `DV01 ${fmtSigned(preview.movedRisk.dv01, 0)}`
                      : `Δ ${fmtSigned(preview.movedRisk.delta, 0)}`
                  }
                />
                <Stat
                  label="Realised P&L (source)"
                  value={fmtSigned(preview.realizedPnlSource, 0)}
                  tone={preview.realizedPnlSource === 0 ? "muted" : "warn"}
                />
                <Stat label="Price" value={preview.transferPrice.toFixed(2)} />
              </div>
              <div className={styles.previewBa}>
                <div className={styles.baRow}>
                  <span className={styles.baName}>{sourceRow?.name ?? "source"}</span>
                  <span className={styles.mono}>{fmtCompact(sourceRow?.netNotional ?? 0)}</span>
                  <span className={styles.baArrow} aria-hidden>
                    →
                  </span>
                  <span className={`${styles.mono} ${styles.baDown}`} data-testid="xfer-source-after">
                    {fmtCompact(preview.sourceNetAfter)}
                  </span>
                </div>
                {targetBookId.length > 0 && (
                  <div className={styles.baRow}>
                    <span className={styles.baName}>{targetRow?.name ?? "target"}</span>
                    <span className={styles.mono}>{fmtCompact(targetRow?.netNotional ?? 0)}</span>
                    <span className={styles.baArrow} aria-hidden>
                      →
                    </span>
                    <span className={`${styles.mono} ${styles.baUp}`} data-testid="xfer-target-after">
                      {fmtCompact(preview.targetNetAfter)}
                    </span>
                  </div>
                )}
              </div>
            </div>
          )}

          {submitError && (
            <p className={styles.error} role="alert" data-testid="xfer-error">
              {submitError}
            </p>
          )}

          {result && (
            <div
              className={`${styles.result} ${result.state === "BOOKED" ? styles.resultBooked : styles.resultPending}`}
              role="status"
              data-testid="xfer-result"
            >
              <strong>
                {result.state === "BOOKED"
                  ? "Booked — risk moved."
                  : "Pending — awaiting acceptance in the counterparty's inbox."}
              </strong>{" "}
              <span className={styles.resultId}>#{result.id}</span>
              {result.state === "BOOKED" && afterMove?.source && afterMove.target && (
                <p className={styles.resultMove}>
                  {afterMove.source.name} net → {fmtCompact(afterMove.source.netNotional)} ·{" "}
                  {afterMove.target.name} net → {fmtCompact(afterMove.target.netNotional)}. See the
                  Risk Dashboard.
                </p>
              )}
            </div>
          )}

          <button
            type="submit"
            className={styles.submit}
            disabled={!canSubmit}
            data-testid="xfer-submit"
          >
            {submitting ? "Submitting…" : `Initiate ${kind.replace(/_/g, " ").toLowerCase()}`}
          </button>
        </section>
      </form>
    </div>
  );
}

/** One labelled preview stat. */
function Stat({
  label,
  value,
  tone,
}: {
  label: string;
  value: string;
  tone?: "muted" | "warn";
}): React.ReactElement {
  return (
    <div className={styles.stat}>
      <span className={styles.statLabel}>{label}</span>
      <span
        className={[styles.statValue, styles.mono, tone === "muted" ? styles.muted : "", tone === "warn" ? styles.warnText : ""]
          .filter(Boolean)
          .join(" ")}
      >
        {value}
      </span>
    </div>
  );
}
