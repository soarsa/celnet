/**
 * RatesRiskPanel — the fixed-income (rates) LENS of the shared, class-parametric
 * RiskWorkspace (`fe-fi-migration`). This is the former standalone `RatesRisk`
 * silo, folded into the ONE Risk workspace as an asset-class lens rather than a
 * peer FI-only workspace — a rates book is now RISKED through the same workflow
 * as FX/options, under a fixed-income license (per claim cl_1f5e26efffee1360: FI
 * is integrated, not a peer). The panel is unchanged in behaviour: a trader
 * assembles a small book of overnight-indexed swaps (an editable position table)
 * and the desk's netted rates risk rolls up live: one card per settlement
 * currency with net PV / PV01 / DV01 and a key-rate DV01 ladder across the curve
 * pillars.
 *
 * The pure request/scope/seed helpers below stay exported (the wire is UNCHANGED
 * by central-core Phase-B — `price_rates_via_contract == price_rates` byte-for-
 * byte — so the `aggregateRatesRisk` seam and its `test/ratesRiskWorkspace.test.ts`
 * are untouched by this move).
 *
 * One contract, two transports (GUI-DESIGN §6.2): the panel talks ONLY to the
 * `CelnetTransport.aggregateRatesRisk` seam, so the SAME portfolio rolls up
 * through the deterministic in-app source (a genuine in-browser OIS bootstrap +
 * additive per-ccy netting, `src/data/mockSource.ts` → `src/data/ratesPricing.ts`)
 * and through the live `RiskService.AggregateRatesRisk` edge — and cannot drift
 * from the wire contract. Transport, conventions and entitlement principal are
 * obtained exactly as RiskWorkspace / BookWorkspace do: `app.transport`,
 * `app.conventions`, and `principalForScope(app.scope)` (grant-all today ⇒ the
 * principal is omitted and the server applies its audited grant-all default).
 *
 * The positions are GENUINE user-editable inputs (seeded from the curve pillars),
 * never hardcoded results; every number on the right is computed by the transport.
 */

import { useCallback, useMemo, useRef } from "react";
import { useApp } from "../app/AppContext";
import { Button } from "../components/Button";
import { Panel } from "../components/Panel";
import { cacheKeyPart, useCachedResource } from "../hooks/useCachedResource";
import { useTableUiState } from "../hooks/useTableUiState";
import { useDebouncedValue } from "../hooks/useDebouncedValue";
import { principalForScope } from "../data/riskView";
import { DEFAULT_USD_SOFR_CURVE } from "../data/ratesPricing";
import { fmtPnlAdaptive } from "../lib/format";
import { KeyRateLadder } from "../viz/KeyRateLadder";
import type {
  AggregateRatesRiskRequest,
  EntitlementPrincipal,
  OisDirection,
  RatesCurveSet,
  RatesPosition,
  RatesRiskNode,
  RatesRiskScope,
} from "../data/contract";
import { pillarYears } from "../data/contract";
import styles from "./RatesRiskWorkspace.module.css";

import { NumberField } from "../components/NumberField";

/** One million — the notional input is denominated in millions of the curve ccy. */
const MM = 1_000_000;

// ---------------------------------------------------------------------------
// pure helpers (exported for unit testing — no React, no transport)
// ---------------------------------------------------------------------------

/** One editable OIS position row in the portfolio table (the GUI input model). */
export interface RatesRiskRow {
  /** Stable React key — NOT sent on the wire. */
  readonly id: string;
  /** Legal-entity id the position books into (a scope filter dimension). */
  readonly entity: number;
  /** Trading-book id the position books into (a scope filter dimension). */
  readonly book: number;
  /** Swap tenor in whole years from spot (`>= 1`). */
  readonly tenorYears: number;
  /** Fixed-leg coupon in percent (4.05 = 4.05%). */
  readonly fixedRatePct: number;
  /** Notional in millions of the curve currency (always positive). */
  readonly notionalMm: number;
  /** Pay-fixed (payer) or receive-fixed (receiver). */
  readonly direction: OisDirection;
}

/** The optional pre-rollup scope filter, as raw text-field inputs. */
export interface RatesRiskScopeInput {
  readonly entity: string;
  readonly book: string;
  readonly ccy: string;
}

/** A seed template anchored to a curve pillar; the coupon is `couponOffsetBp` off par. */
interface SeedTemplate {
  readonly tenorYears: number;
  readonly entity: number;
  readonly book: number;
  readonly notionalMm: number;
  readonly direction: OisDirection;
  /** Coupon offset from the pillar par rate, in basis points (signed). */
  readonly couponOffsetBp: number;
}

/**
 * The seed book: a handful of swaps anchored to the calibrating curve pillars,
 * spread across two entities / three books and both directions, each struck a few
 * bp off its pillar par so the netted PV is real (not a degenerate zero). These
 * are editable INPUTS derived from the curve pillars — never baked-in results.
 */
const SEED_TEMPLATES: readonly SeedTemplate[] = [
  {
    tenorYears: 2,
    entity: 1,
    book: 10,
    notionalMm: 50,
    direction: "RECEIVE_FIXED",
    couponOffsetBp: -8,
  },
  {
    tenorYears: 5,
    entity: 1,
    book: 10,
    notionalMm: 100,
    direction: "PAY_FIXED",
    couponOffsetBp: -5,
  },
  {
    tenorYears: 10,
    entity: 1,
    book: 20,
    notionalMm: 75,
    direction: "RECEIVE_FIXED",
    couponOffsetBp: 10,
  },
  {
    tenorYears: 30,
    entity: 2,
    book: 30,
    notionalMm: 25,
    direction: "PAY_FIXED",
    couponOffsetBp: 0,
  },
];

/** Round a percent figure to bp precision (4 decimal places of a percent). */
function roundPct(pct: number): number {
  return Number(pct.toFixed(4));
}

/**
 * Build the seed portfolio rows from a curve set's pillars: each template's
 * coupon is the matched pillar's par rate plus the template's bp offset, so the
 * seed is genuinely derived from the live curve (not a constant).
 */
export function defaultRatesRiskRows(
  curve: RatesCurveSet = DEFAULT_USD_SOFR_CURVE,
): RatesRiskRow[] {
  return SEED_TEMPLATES.map((t, i) => {
    const pillar = curve.pillars.find(
      (p) => pillarYears(p.tenor) === t.tenorYears,
    );
    const parPct = (pillar ? pillar.parRate : 0.04) * 100;
    return {
      id: `seed-${t.tenorYears}y-${i}`,
      entity: t.entity,
      book: t.book,
      tenorYears: t.tenorYears,
      fixedRatePct: roundPct(parPct + t.couponOffsetBp / 100),
      notionalMm: t.notionalMm,
      direction: t.direction,
    };
  });
}

/**
 * Project one editable row onto a wire {@link RatesPosition}: percent → decimal
 * rate, millions → absolute notional, the row index → the informational
 * `positionId` echo. The direction sign lives on the instrument, so the rollup
 * nets long and short books by sign.
 */
export function rowToPosition(row: RatesRiskRow, index: number): RatesPosition {
  return {
    positionId: BigInt(index),
    entity: row.entity,
    book: row.book,
    instrument: {
      tenorYears: row.tenorYears,
      fixedRate: row.fixedRatePct / 100,
      notional: row.notionalMm * MM,
      direction: row.direction,
    },
  };
}

/**
 * Build the optional `(entity, book, ccy)` scope from raw text inputs: each blank
 * / non-numeric entity-or-book field and each blank ccy field is omitted, so an
 * empty filter returns `undefined` (the whole portfolio contributes).
 */
export function buildScope(
  input: RatesRiskScopeInput,
): RatesRiskScope | undefined {
  const scope: RatesRiskScope = {};
  const entity = Number.parseInt(input.entity, 10);
  if (input.entity.trim() !== "" && Number.isFinite(entity))
    scope.entity = entity;
  const book = Number.parseInt(input.book, 10);
  if (input.book.trim() !== "" && Number.isFinite(book)) scope.book = book;
  const ccy = input.ccy.trim();
  if (ccy !== "") scope.ccy = ccy;
  return Object.keys(scope).length > 0 ? scope : undefined;
}

/** Optional inputs to {@link buildRatesRiskRequest} beyond the position rows. */
export interface RatesRiskRequestOptions {
  /** The shared curve set every position prices against (default USD-SOFR). */
  readonly curve?: RatesCurveSet;
  /** The optional pre-rollup `(entity, book, ccy)` filter. */
  readonly scope?: RatesRiskScope;
  /** The entitlement principal (omitted ⇒ the audited grant-all default). */
  readonly principal?: EntitlementPrincipal;
  /** Optional client correlation echo. */
  readonly correlationId?: bigint;
}

/**
 * Assemble the `AggregateRatesRiskRequest` from the editable rows: each row
 * becomes a position, the optional scope / principal / correlation echo are
 * threaded only when present. Pure — the React layer just hands it to the
 * transport.
 */
export function buildRatesRiskRequest(
  rows: readonly RatesRiskRow[],
  opts: RatesRiskRequestOptions = {},
): AggregateRatesRiskRequest {
  const request: AggregateRatesRiskRequest = {
    curveSet: opts.curve ?? DEFAULT_USD_SOFR_CURVE,
    positions: rows.map(rowToPosition),
  };
  if (opts.scope !== undefined) request.scope = opts.scope;
  if (opts.principal !== undefined) request.principal = opts.principal;
  if (opts.correlationId !== undefined)
    request.correlationId = opts.correlationId;
  return request;
}

/** The largest absolute bucket DV01 of a node's ladder (floored, for bar scaling). */
export function ladderMaxAbs(node: RatesRiskNode): number {
  return node.keyRateLadder.reduce(
    (m, k) => Math.max(m, Math.abs(k.dv01)),
    1e-9,
  );
}

// ---------------------------------------------------------------------------
// the workspace
// ---------------------------------------------------------------------------

/** Debounce (ms) before re-aggregating after a portfolio edit. */
const REPRICE_DEBOUNCE_MS = 220;

export function RatesRiskPanel(): React.ReactElement {
  const app = useApp();
  const curve = DEFAULT_USD_SOFR_CURVE;

  // The editable portfolio + scope filter are persisted per-table so a trader's
  // edited book (and scope) SURVIVE a tab switch (the workspace unmounting) rather
  // than reseeding to the defaults on return.
  const [ui, setUi] = useTableUiState<{
    rows: RatesRiskRow[];
    scopeInput: RatesRiskScopeInput;
  }>("fi-rates-risk", {
    rows: defaultRatesRiskRows(curve),
    scopeInput: { entity: "", book: "", ccy: "" },
  });
  const rows = ui.rows;
  const scopeInput = ui.scopeInput;
  // The latest rows/scope through refs so the functional row mutations below read the
  // current value (they merge into the persisted store, which has no functional patch).
  const rowsRef = useRef(rows);
  rowsRef.current = rows;
  const setRows = useCallback(
    (next: RatesRiskRow[] | ((rs: RatesRiskRow[]) => RatesRiskRow[])) => {
      const value = typeof next === "function" ? next(rowsRef.current) : next;
      setUi({ rows: value });
    },
    [setUi],
  );
  const setScopeInput = useCallback(
    (next: RatesRiskScopeInput | ((s: RatesRiskScopeInput) => RatesRiskScopeInput)) => {
      const value = typeof next === "function" ? next(ui.scopeInput) : next;
      setUi({ scopeInput: value });
    },
    [setUi, ui.scopeInput],
  );
  // Monotone id source for rows the trader adds (seed rows carry `seed-*` ids).
  const nextId = useRef(0);

  // The transport label tells the trader which engine rolled this up (offline
  // in-app rollup vs the live `AggregateRatesRisk` edge); both net the SAME book.
  const isOffline = !app.transport.label.startsWith("live");

  // The entitlement principal — obtained exactly as RiskWorkspace/BookWorkspace do
  // (`principalForScope(app.scope)`); grant-all today ⇒ `undefined` ⇒ the request
  // omits it and the server applies its audited grant-all default.
  const principal = useMemo(() => principalForScope(app.scope), [app.scope]);

  // Debounce the editable portfolio so re-aggregation only fires once edits settle
  // (the same intent the prior in-effect debounce had), then build the request off the
  // settled inputs.
  const debouncedRows = useDebouncedValue(rows, REPRICE_DEBOUNCE_MS);
  const debouncedScopeInput = useDebouncedValue(scopeInput, REPRICE_DEBOUNCE_MS);
  const scope = useMemo(() => buildScope(debouncedScopeInput), [debouncedScopeInput]);

  const request = useMemo<AggregateRatesRiskRequest>(
    () =>
      buildRatesRiskRequest(debouncedRows, {
        curve,
        ...(scope ? { scope } : {}),
        ...(principal ? { principal } : {}),
      }),
    [debouncedRows, curve, scope, principal],
  );

  // Stale-while-revalidate rollup: cache the aggregation keyed on the projected
  // positions + scope + principal, so returning to this workspace shows the last
  // rollup INSTANTLY (no "Aggregating…" flash) while a background revalidation
  // refreshes it. A failure surfaces as a real error and keeps the last good rollup —
  // never a fabricated one.
  const cacheKey = `ratesRisk|${cacheKeyPart(request.positions)}|${cacheKeyPart(scope)}|${cacheKeyPart(principal)}`;
  const {
    data: nodesData,
    isValidating,
    error: fetchError,
  } = useCachedResource<RatesRiskNode[]>(cacheKey, () =>
    app.transport.aggregateRatesRisk(request, app.conventions).then((res) => [...res.nodes]),
  );
  // `null` (not yet loaded) drives the ResultsBody "Aggregating…" state; a landed
  // rollup renders. `busy` reflects a background revalidation.
  const nodes = nodesData ?? null;
  const busy = isValidating;
  const error =
    fetchError === undefined || fetchError === null
      ? null
      : fetchError instanceof Error
        ? fetchError.message
        : "rates risk aggregation failed";

  const updateRow = useCallback(
    (id: string, patch: Partial<RatesRiskRow>) => {
      setRows((rs) => rs.map((r) => (r.id === id ? { ...r, ...patch } : r)));
    },
    [setRows],
  );

  const addRow = useCallback(() => {
    setRows((rs) => [
      ...rs,
      {
        id: `row-${nextId.current++}`,
        entity: 1,
        book: 10,
        tenorYears: 5,
        fixedRatePct: roundPct(
          curve.pillars[3]?.parRate ? curve.pillars[3].parRate * 100 : 4.05,
        ),
        notionalMm: 25,
        direction: "RECEIVE_FIXED",
      },
    ]);
  }, [curve.pillars, setRows]);

  const removeRow = useCallback(
    (id: string) => {
      setRows((rs) => rs.filter((r) => r.id !== id));
    },
    [setRows],
  );

  const clearScope = useCallback(
    () => setScopeInput({ entity: "", book: "", ccy: "" }),
    [setScopeInput],
  );

  const scopeActive = scope !== undefined;

  return (
    <div className={styles.wrap}>
      <Panel material="float" className={styles.builder} title="Portfolio">
        <div className={styles.curveRow}>
          <span className={styles.curveLabel}>Curve</span>
          <span className={styles.curveName}>{curve.currency}-SOFR</span>
          <span className={styles.curveMeta}>
            {curve.pillars.length} pillars · ref {curve.referenceDate.year}-
            {String(curve.referenceDate.month).padStart(2, "0")}-
            {String(curve.referenceDate.day).padStart(2, "0")} ·
            self-discounting
          </span>
          <span className={styles.engine}>
            {isOffline ? "in-app rollup" : "live edge"}
          </span>
        </div>

        <div className={styles.tableWrap}>
          <table className={styles.table}>
            <thead>
              <tr>
                <th className={styles.thNum}>Entity</th>
                <th className={styles.thNum}>Book</th>
                <th className={styles.thNum}>Tenor</th>
                <th className={styles.thNum}>Fixed %</th>
                <th className={styles.thNum}>Notional</th>
                <th className={styles.thSide}>Side</th>
                <th className={styles.thAct} aria-label="remove" />
              </tr>
            </thead>
            <tbody>
              {rows.map((row) => (
                <tr key={row.id}>
                  <td>
                    <NumberField
                      className={styles.cellInput}
                      min={0}
                      step={1}
                      value={row.entity}
                      aria-label="position entity id"
                      onChange={(e) =>
                        updateRow(row.id, {
                          entity: Math.trunc(Number(e.target.value)),
                        })
                      }
                    />
                  </td>
                  <td>
                    <NumberField
                      className={styles.cellInput}
                      min={0}
                      step={1}
                      value={row.book}
                      aria-label="position book id"
                      onChange={(e) =>
                        updateRow(row.id, {
                          book: Math.trunc(Number(e.target.value)),
                        })
                      }
                    />
                  </td>
                  <td>
                    <NumberField
                      className={styles.cellInput}
                      min={1}
                      step={1}
                      value={row.tenorYears}
                      aria-label="swap tenor in years"
                      onChange={(e) =>
                        updateRow(row.id, {
                          tenorYears: Math.trunc(Number(e.target.value)),
                        })
                      }
                    />
                  </td>
                  <td>
                    <NumberField
                      className={styles.cellInput}
                      step={0.01}
                      value={row.fixedRatePct}
                      aria-label="fixed rate in percent"
                      onChange={(e) =>
                        updateRow(row.id, {
                          fixedRatePct: Number(e.target.value),
                        })
                      }
                    />
                  </td>
                  <td>
                    <NumberField
                      className={styles.cellInput}
                      min={0}
                      step={5}
                      value={row.notionalMm}
                      aria-label="notional in millions"
                      onChange={(e) =>
                        updateRow(row.id, {
                          notionalMm: Number(e.target.value),
                        })
                      }
                    />
                  </td>
                  <td>
                    <select
                      className={styles.cellSelect}
                      value={row.direction}
                      aria-label="swap direction"
                      onChange={(e) =>
                        updateRow(row.id, {
                          direction: e.target.value as OisDirection,
                        })
                      }
                    >
                      <option value="RECEIVE_FIXED">Receive</option>
                      <option value="PAY_FIXED">Pay</option>
                    </select>
                  </td>
                  <td className={styles.actCell}>
                    <button
                      type="button"
                      className={styles.rowRemove}
                      onClick={() => removeRow(row.id)}
                      title="remove position"
                      aria-label="remove position"
                      disabled={rows.length === 1}
                    >
                      ×
                    </button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>

        <div className={styles.tableFoot}>
          <Button variant="secondary" onClick={addRow}>
            + Add position
          </Button>
          <span className={styles.rowCount}>
            {rows.length} position{rows.length === 1 ? "" : "s"}
          </span>
        </div>

        <fieldset className={styles.scope}>
          <legend className={styles.scopeLegend}>
            Scope filter
            {scopeActive && (
              <button
                type="button"
                className={styles.scopeClear}
                onClick={clearScope}
              >
                clear
              </button>
            )}
          </legend>
          <label className={styles.scopeField}>
            <span className={styles.scopeFieldLabel}>Entity</span>
            <NumberField
              className={styles.scopeInput}
              min={0}
              step={1}
              placeholder="all"
              value={scopeInput.entity}
              aria-label="filter by entity"
              onChange={(e) =>
                setScopeInput((s) => ({ ...s, entity: e.target.value }))
              }
            />
          </label>
          <label className={styles.scopeField}>
            <span className={styles.scopeFieldLabel}>Book</span>
            <NumberField
              className={styles.scopeInput}
              min={0}
              step={1}
              placeholder="all"
              value={scopeInput.book}
              aria-label="filter by book"
              onChange={(e) =>
                setScopeInput((s) => ({ ...s, book: e.target.value }))
              }
            />
          </label>
          <label className={styles.scopeField}>
            <span className={styles.scopeFieldLabel}>Ccy</span>
            <input
              className={styles.scopeInput}
              type="text"
              placeholder="all"
              maxLength={3}
              value={scopeInput.ccy}
              aria-label="filter by settlement currency"
              onChange={(e) =>
                setScopeInput((s) => ({
                  ...s,
                  ccy: e.target.value.toUpperCase(),
                }))
              }
            />
          </label>
        </fieldset>

        {error && (
          <p className={styles.error} role="alert">
            {error}
          </p>
        )}
      </Panel>

      <Panel className={styles.results} title="Netted rates risk">
        <ResultsBody
          nodes={nodes}
          busy={busy}
          curve={curve}
          scopeActive={scopeActive}
        />
      </Panel>
    </div>
  );
}

/** The right-hand rollup: per-ccy node cards, with explicit empty / loading states. */
function ResultsBody({
  nodes,
  busy,
  curve,
  scopeActive,
}: {
  nodes: RatesRiskNode[] | null;
  busy: boolean;
  curve: RatesCurveSet;
  scopeActive: boolean;
}): React.ReactElement {
  if (nodes === null) {
    return <p className={styles.empty}>Aggregating portfolio risk…</p>;
  }
  if (nodes.length === 0) {
    return (
      <p className={styles.empty}>
        {scopeActive
          ? "No positions match the scope filter — widen or clear the scope."
          : "Add a position to roll up the desk's netted rates risk."}
      </p>
    );
  }
  return (
    <div className={styles.nodes} aria-busy={busy}>
      {nodes.map((node) => (
        <NodeCard key={node.ccy} node={node} curve={curve} />
      ))}
    </div>
  );
}

/** One settlement-currency rollup: net measures + the key-rate DV01 ladder strip. */
function NodeCard({
  node,
  curve,
}: {
  node: RatesRiskNode;
  curve: RatesCurveSet;
}): React.ReactElement {
  // Project the node's wire ladder onto the KeyRateLadder viz: one signed rung per
  // calibrating pillar, reconciled against the node's parallel net DV01 (the same
  // additive identity the offline core is held to). The lib component owns the band
  // scale, the diverging-ramp colour, and the Σ reconciliation annotation.
  const pillars = node.keyRateLadder.map((bucket) => ({
    pillar: `${bucket.tenorYears}y`,
    dv01: bucket.dv01,
  }));

  return (
    <article className={styles.nodeCard}>
      <header className={styles.nodeHead}>
        <span className={styles.nodeCcy}>{node.ccy}</span>
        <span className={styles.nodeTag}>
          {node.keyRateLadder.length} pillars · {curve.currency}-SOFR
        </span>
      </header>

      <dl className={styles.metrics}>
        <Metric
          label="Net PV"
          value={fmtPnlAdaptive(node.netPv)}
          unit={node.ccy}
          emphatic
        />
        <Metric
          label="Net PV01"
          value={fmtPnlAdaptive(node.netPv01)}
          unit={`${node.ccy}/bp`}
        />
        <Metric
          label="Net DV01"
          value={fmtPnlAdaptive(node.netDv01)}
          unit={`${node.ccy}/bp`}
        />
      </dl>

      <KeyRateLadder
        data={pillars}
        parallelDv01={node.netDv01}
        unit={`${node.ccy}/bp`}
      />
    </article>
  );
}

/** One headline measure: a labelled term/value pair in a node card. */
function Metric({
  label,
  value,
  unit,
  emphatic,
}: {
  label: string;
  value: string;
  unit?: string;
  emphatic?: boolean;
}): React.ReactElement {
  return (
    <div
      className={`${styles.metric} ${emphatic ? styles.metricEmphatic : ""}`}
    >
      <dt className={styles.metricLabel}>{label}</dt>
      <dd className={styles.metricValue}>
        {value}
        {unit && <span className={styles.metricUnit}>{unit}</span>}
      </dd>
    </div>
  );
}

export { RatesRiskPanel as RatesRiskWorkspace };
