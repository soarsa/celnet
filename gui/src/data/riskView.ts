/**
 * riskView — the thin client-side glue between the toolbar Scope and the SERVER's
 * `RiskService` (src/data/contract.RiskService messages). It does NOT aggregate:
 * aggregation is owned by the server (GUIDE.md rule 11 / API-first parity). This
 * module only translates the GUI's `ScopeContext` into the contract's
 * `RiskDimension` + `EntitlementPrincipal`, and assembles the `ReportingNumeraire`
 * from the watched-pairs' live spot so the server can collapse every leg into one
 * common reporting currency (resolving the old "native premium units" caveat).
 *
 * The Book view then issues ONE `aggregate_risk` call for the rolled-up node tree
 * and a `drill_risk` for the Book→Risk drill — never a per-position loop.
 */

import type {
  EntitlementPrincipal,
  ReportingNumeraire,
  RiskDimension,
  RiskScope,
  RiskVegaPillar,
} from "./contract";
import type { ScopeContext, ScopeLevel } from "../app/AppContext";
import type { PairContext } from "./seed";

/**
 * The reporting currency the Book collapses every leg into. USD is the desk's
 * common numeraire; the per-ccy spot rates into it are assembled from the live
 * watched-pair markets (so the numeraire is real market data, never a constant).
 */
export const REPORTING_CCY = "USD";

/**
 * The org dimension the Book groups by, derived from the active scope. The
 * toolbar scope path (Firm · Desk · Book · Pair) selects how the rolled-up tree
 * is sliced:
 *   - at the Firm root we group by CCY_PAIR (the desk's natural risk slices);
 *   - scoped to a Desk we group by BOOK; to a Book we group by TRADER; to a Pair
 *     we group by CCY_PAIR (the leaf slice).
 * This is the contract's orthogonal `RiskDimension` — a way to slice the SAME
 * cube, chosen by where the trader is standing.
 */
export function dimensionForScope(scope: ScopeContext): RiskDimension {
  const tail = scope.path[scope.path.length - 1]?.level ?? "firm";
  return DIMENSION_BY_LEVEL[tail];
}

const DIMENSION_BY_LEVEL: Record<ScopeLevel, RiskDimension> = {
  firm: "CCY_PAIR",
  desk: "BOOK",
  book: "TRADER",
  pair: "CCY_PAIR",
};

/**
 * The limit scope for the active toolbar scope — the ORG node the trader stands
 * at (Firm/Desk/Book/Trader). Distinct from the Book group-by `dimension`: limits
 * are read at a single org node, never at the `CCY_PAIR` group-by axis (the server
 * rejects a bare `CCY_PAIR` limit scope — its `u64` value cannot reconstruct the
 * pair). The `value` is 0 at the firm root (the apex node); a real drill carries
 * the resolved ancestor handle.
 */
export function limitScopeForScope(scope: ScopeContext): RiskScope {
  const tail = scope.path[scope.path.length - 1]?.level ?? "firm";
  return { dimension: LIMIT_DIMENSION_BY_LEVEL[tail], value: 0n };
}

const LIMIT_DIMENSION_BY_LEVEL: Record<ScopeLevel, RiskDimension> = {
  firm: "FIRM",
  desk: "DESK",
  book: "BOOK",
  // A pair-scoped view still reads limits at the firm node (a pair is not an org
  // node the limit tree keys on; the bare CCY_PAIR scope is rejected server-side).
  pair: "FIRM",
};

/**
 * The entitlement principal for the active scope. Today the GUI is `grant-all`
 * (show-all-now), so a request OMITS the principal and the server treats it as
 * grant-all. When a real entitlement predicate lands, this returns the principal
 * the server prunes by BEFORE roll-up — the call sites already pass it through, so
 * the wiring is zero-rework.
 */
export function principalForScope(scope: ScopeContext): EntitlementPrincipal | undefined {
  // Today the GUI scope is exactly `grant-all` (show-all-now), so we OMIT the
  // principal and the server applies its grant-all default. When a real predicate
  // lands, `scope.principal` widens and this maps it to an `EntitlementPrincipal`;
  // the call sites already thread the (optional) principal through, so it is
  // zero-rework. The `void` keeps the parameter live for that evolution.
  void scope;
  return undefined;
}

/**
 * Build the `ReportingNumeraire` (USD) from the watched-pair markets. For each
 * pair we know one leg's spot rate into the other; we derive every traded ccy's
 * rate into USD:
 *   - a USD-quote pair (EUR/USD) gives base→USD = spot directly;
 *   - a USD-base pair (USD/JPY) gives quote→USD = 1/spot;
 * USD itself is implicitly 1.0. A ccy with no path to USD is simply omitted — the
 * server fails loudly if an aggregated leg needs a missing rate, so the caller
 * surfaces that honestly rather than the GUI inventing a rate.
 */
export function reportingNumeraire(pairs: PairContext[]): ReportingNumeraire {
  const rateInto = new Map<string, number>();
  rateInto.set(REPORTING_CCY, 1);
  for (const { pair, market } of pairs) {
    if (!(market.spot > 0)) continue;
    if (pair.quote === REPORTING_CCY) {
      // base→USD = spot (USD per 1 base).
      rateInto.set(pair.base, market.spot);
    } else if (pair.base === REPORTING_CCY) {
      // quote→USD = 1/spot (USD per 1 quote).
      rateInto.set(pair.quote, 1 / market.spot);
    }
  }
  // Second pass: a cross pair (no USD leg) is bridged through a ccy we now know.
  for (const { pair, market } of pairs) {
    if (!(market.spot > 0)) continue;
    if (pair.base === REPORTING_CCY || pair.quote === REPORTING_CCY) continue;
    const baseUsd = rateInto.get(pair.base);
    const quoteUsd = rateInto.get(pair.quote);
    if (baseUsd === undefined && quoteUsd !== undefined) {
      // base→USD = (quote per base via spot)×(quote→USD) = spot×quoteUsd.
      rateInto.set(pair.base, market.spot * quoteUsd);
    } else if (quoteUsd === undefined && baseUsd !== undefined) {
      rateInto.set(pair.quote, baseUsd / market.spot);
    }
  }
  const rates = [...rateInto.entries()]
    .filter(([ccy]) => ccy !== REPORTING_CCY)
    .map(([ccy, rate]) => ({ ccy, rate }))
    .sort((a, b) => a.ccy.localeCompare(b.ccy));
  return { numeraire: REPORTING_CCY, rates };
}

/**
 * The default vega-ladder pillar grid the Book requests (tenor in DAYS, delta in
 * BASIS POINTS, per the contract `RiskVegaPillar`). The standard desk pillars
 * across the common tenors. Empty would let the server pick its default grid; we
 * send an explicit grid so the ladder is stable across server builds.
 */
export const BOOK_VEGA_PILLARS: RiskVegaPillar[] = (() => {
  const tenorsDays = [7, 30, 60, 91, 182, 365];
  const deltaBp = [5000, 2500, -2500, 1000, -1000]; // ATM, ±25Δ, ±10Δ
  const grid: RiskVegaPillar[] = [];
  for (const tenorDays of tenorsDays) {
    for (const d of deltaBp) grid.push({ tenorDays, deltaBp: d });
  }
  return grid;
})();
