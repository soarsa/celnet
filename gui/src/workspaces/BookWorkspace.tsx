/**
 * BookWorkspace — the firm-scale HIERARCHICAL risk view, computed SERVER-SIDE.
 *
 * Aggregation is owned by the server (CLAUDE.md rule 11 / API-first parity): this
 * view issues ONE `aggregate_risk` call for the rolled-up node tree over the org
 * dimension the active Scope selects, plus a `drill_risk` for the Book→Risk drill
 * — it NEVER loops positions and sums client-side (the old `portfolioRisk`
 * per-position `transport.scenario` loop, now deleted). Every number is the
 * server's, rolled up across the entitled book and collapsed into ONE common
 * reporting numeraire (USD) via `celnet-risk-normalize` — so the old "native
 * premium units" caveat is RESOLVED: a high-spot pair no longer dominates a raw
 * sum, because every leg is already in common units.
 *
 * The Scope toolbar drives the group-by dimension and the entitlement principal
 * (grant-all today, show-all-now). A node row drills into its constituents in
 * Risk; the Limits panel surfaces `celnet-limits` utilization/RAG for the scope.
 */

import { useEffect, useMemo, useState } from "react";
import { useApp } from "../app/AppContext";
import type {
  AdditiveRisk,
  AggregateRiskResponse,
  Instrument,
  LimitStatusResponse,
  LimitUtilization,
  RiskDimension,
  RiskNode,
  RiskPosition,
} from "../data/contract";
import {
  BOOK_VEGA_PILLARS,
  dimensionForScope,
  limitScopeForScope,
  principalForScope,
  reportingNumeraire,
  REPORTING_CCY,
} from "../data/riskView";
import { Panel } from "../components/Panel";
import { fmtPnlAdaptive } from "../lib/format";
import { tenorLabel } from "../lib/trend";
import styles from "./BookWorkspace.module.css";

/** The fetched server result for the active scope (aggregate + limits). */
interface BookView {
  aggregate: AggregateRiskResponse;
  limits: LimitStatusResponse;
}

export function BookWorkspace(): React.ReactElement {
  const app = useApp();
  const [view, setView] = useState<BookView | null>(null);
  const [error, setError] = useState<string | null>(null);

  const dimension = useMemo(() => dimensionForScope(app.scope), [app.scope]);
  const limitScope = useMemo(() => limitScopeForScope(app.scope), [app.scope]);
  const principal = useMemo(() => principalForScope(app.scope), [app.scope]);
  // The reporting numeraire (USD) is assembled from the live watched-pair spots,
  // so the server collapses every leg into real common units — not a constant.
  const numeraire = useMemo(() => reportingNumeraire(app.pairs), [app.pairs]);

  useEffect(() => {
    let live = true;
    setError(null);
    const run = async (): Promise<void> => {
      // ONE rolled-up aggregate over the org dimension + the scope's limit tree.
      // Aggregation + limit evaluation happen server-side; the client only asks.
      const [aggregate, limits] = await Promise.all([
        app.transport.aggregateRisk({
          dimension,
          numeraire,
          ...(principal ? { principal } : {}),
          vegaPillars: BOOK_VEGA_PILLARS,
          varSpotShocks: [],
          varAlpha: 0,
          curvatureRiskWeight: 0,
        }),
        app.transport.limitStatus({
          scope: limitScope,
          numeraire,
          ...(principal ? { principal } : {}),
          vegaPillars: BOOK_VEGA_PILLARS,
          varSpotShocks: [],
          varAlpha: 0,
        }),
      ]);
      if (live) setView({ aggregate, limits });
    };
    void run().catch((e: unknown) => {
      if (live) setError(e instanceof Error ? e.message : String(e));
    });
    return () => {
      live = false;
    };
  }, [app.transport, dimension, limitScope, numeraire, principal]);

  // Book→Risk drill: a node row opens its largest contributing position in Risk.
  // We ask the server for the node's positions (drill_risk include_positions);
  // Book and Risk are the same cube at two zooms.
  const drillNode = (node: RiskNode): void => {
    void app.transport
      .drillRisk({
        node: { dimension: node.dimension, value: node.group },
        childDimension: node.dimension,
        numeraire,
        ...(principal ? { principal } : {}),
        vegaPillars: BOOK_VEGA_PILLARS,
        includeChildren: false,
        includePositions: true,
      })
      .then((res) => {
        const pos = res.positions
          .slice()
          .sort((a, b) => Math.abs(b.notionalBase) - Math.abs(a.notionalBase))[0];
        if (!pos) return;
        const instrument = positionToInstrument(pos);
        const cohort = res.positions.length > 1 ? ` (largest of ${res.positions.length})` : "";
        app.drillToRisk(instrument, `${nodeLabel(node)} ${optionShort(pos)}${cohort}`);
      })
      .catch(() => {
        // A drill that the server cannot resolve (e.g. no flat positions for the
        // node) leaves the Book in place — honest, no fabricated selection.
      });
  };

  if (error !== null) {
    return (
      <div className={styles.empty}>
        <span className={styles.emptyGlyph}>Σ</span>
        <p className={styles.emptyTitle}>Risk unavailable</p>
        <p className={styles.emptyBody}>
          The server could not aggregate the book for this scope: {error}. This is
          shown honestly rather than a fabricated risk picture; it typically means a
          reporting-numeraire rate is missing for a traded currency, or the edge is
          still starting.
        </p>
      </div>
    );
  }

  if (view === null) {
    return <div className={styles.loading}>Aggregating book risk…</div>;
  }

  const { aggregate, limits } = view;
  const nodes = aggregate.nodes;

  if (nodes.length === 0) {
    return (
      <div className={styles.empty}>
        <span className={styles.emptyGlyph}>Σ</span>
        <p className={styles.emptyTitle}>No open positions</p>
        <p className={styles.emptyBody}>
          The entitled book for this scope is empty. Click-to-trade a streamed price
          or book a ticket; firm-scale aggregated risk appears here the moment a
          position exists. Nothing is shown until there is something real to
          aggregate.
        </p>
      </div>
    );
  }

  // Firm total = the sum of the node additive measures (additive roll-up). The
  // server already returns each node collapsed into the reporting numeraire, so
  // summing the nodes is the firm-level total in common units.
  const total = sumAdditive(nodes.map((n) => n.additive));
  const positionCount = nodes.reduce((acc, n) => acc + n.positionCount, 0);

  const cards = [
    { label: `Net P&L (${REPORTING_CCY})`, value: total.premiumNumeraire },
    { label: `Net Delta (${REPORTING_CCY})`, value: total.deltaNumeraire },
    { label: `Net Vega (${REPORTING_CCY})`, value: total.vegaNumeraire },
    { label: `Net Theta (${REPORTING_CCY})`, value: total.theta },
  ];

  // The aggregate vega ladder is the firm total's per-pillar vega, summed across
  // nodes (each ladder bucket is already in the reporting numeraire).
  const ladder = mergeVegaLadder(nodes);
  const maxVega = Math.max(...ladder.map((b) => Math.abs(b.vega)), 1e-12);

  return (
    <div className={styles.wrap}>
      <div className={styles.cards}>
        {cards.map((c) => (
          <div key={c.label} className={styles.card}>
            <span className={`brand-label ${styles.cardLabel}`}>{c.label}</span>
            <span className={`num ${styles.cardValue} ${signClass(c.value)}`}>
              {fmtPnlAdaptive(c.value)}
            </span>
          </div>
        ))}
      </div>

      <div className={styles.cols}>
        {/* --- rolled-up node breakdown (server group-by) --- */}
        <Panel
          glyph="Σ"
          title={`Book breakdown · ${positionCount} positions · ${nodes.length} ${dimensionNoun(
            dimension,
          )}`}
          className={styles.tablePanel}
        >
          <table className={styles.table}>
            <thead>
              <tr>
                <th className={styles.colPair}>{dimensionHeader(dimension)}</th>
                <th className={`num ${styles.colNum}`}>Pos</th>
                <th className={`num ${styles.colNum}`}>Net Δ</th>
                <th className={`num ${styles.colNum}`}>Net Vega</th>
                <th className={`num ${styles.colNum}`}>Net Gamma</th>
                <th className={`num ${styles.colNum}`}>Net Theta</th>
              </tr>
            </thead>
            <tbody>
              {nodes.map((n) => (
                <tr
                  key={String(n.group)}
                  className={styles.drillRow}
                  onClick={() => drillNode(n)}
                  role="button"
                  tabIndex={0}
                  title={`Drill to Risk · ${nodeLabel(n)}`}
                  onKeyDown={(ev) => {
                    if (ev.key === "Enter" || ev.key === " ") {
                      ev.preventDefault();
                      drillNode(n);
                    }
                  }}
                >
                  <td className={styles.pairCell}>
                    <span className={styles.drillCue} aria-hidden>
                      ›
                    </span>
                    {nodeLabel(n)}
                  </td>
                  <td className="num">{n.positionCount}</td>
                  <td className={`num ${signClass(n.additive.deltaNumeraire)}`}>
                    {fmtPnlAdaptive(n.additive.deltaNumeraire)}
                  </td>
                  <td className={`num ${signClass(n.additive.vegaNumeraire)}`}>
                    {fmtPnlAdaptive(n.additive.vegaNumeraire)}
                  </td>
                  <td className={`num ${signClass(n.additive.gamma)}`}>
                    {fmtPnlAdaptive(n.additive.gamma)}
                  </td>
                  <td className={`num ${signClass(n.additive.theta)}`}>
                    {fmtPnlAdaptive(n.additive.theta)}
                  </td>
                </tr>
              ))}
            </tbody>
            <tfoot>
              <tr>
                <td className={styles.pairCell}>Firm</td>
                <td className="num">{positionCount}</td>
                <td className={`num ${signClass(total.deltaNumeraire)}`}>
                  {fmtPnlAdaptive(total.deltaNumeraire)}
                </td>
                <td className={`num ${signClass(total.vegaNumeraire)}`}>
                  {fmtPnlAdaptive(total.vegaNumeraire)}
                </td>
                <td className={`num ${signClass(total.gamma)}`}>{fmtPnlAdaptive(total.gamma)}</td>
                <td className={`num ${signClass(total.theta)}`}>{fmtPnlAdaptive(total.theta)}</td>
              </tr>
            </tfoot>
          </table>
          <p className={styles.note}>
            Click a row to drill into its largest contributing position in Risk —
            Book and Risk are the same cube at two zooms. Every measure is rolled up
            by the server over the <em>{dimensionNoun(dimension)}</em> dimension and
            collapsed into a single common reporting numeraire (
            <strong>{aggregate.numeraire}</strong>) via the risk-normalisation
            boundary — so cross-pair totals are directly comparable (no pair&apos;s
            native premium units dominate the sum). The net delta is also broken out
            per currency leg in {aggregate.numeraire} terms.
          </p>
        </Panel>

        <div className={styles.sideCol}>
          {/* --- aggregate vega ladder --- */}
          <Panel glyph="ν" title="Aggregate vega ladder" className={styles.ladderPanel}>
            {ladder.length > 0 && ladder.some((b) => Math.abs(b.vega) > 0) ? (
              <div className={styles.ladder}>
                <div className={styles.ladderHead}>
                  <span>Tenor</span>
                  <span>Pillar</span>
                  <span>Σ Vega ({aggregate.numeraire})</span>
                </div>
                {ladder.map((b) => (
                  <div key={`${b.tenorDays}|${b.deltaBp}`} className={styles.ladderRow}>
                    <span className="num">{tenorLabel(b.tenorDays / 365)}</span>
                    <span className="num">{pillarName(b.deltaBp)}</span>
                    <span className={styles.bar}>
                      <span
                        className={styles.barFill}
                        style={{
                          width: `${(Math.abs(b.vega) / maxVega) * 100}%`,
                          background:
                            b.vega >= 0
                              ? "oklch(from var(--bid) l c h / 0.4)"
                              : "oklch(from var(--offer) l c h / 0.4)",
                        }}
                      />
                      <span className={`num ${styles.barVal} ${signClass(b.vega)}`}>
                        {fmtPnlAdaptive(b.vega)}
                      </span>
                    </span>
                  </div>
                ))}
              </div>
            ) : (
              <div className={styles.emptyInline}>
                No bucketed vega returned for the book at the requested pillars.
              </div>
            )}

            <div className={styles.disclosure}>
              <span className={`brand-label ${styles.discLabel}`}>
                net delta by currency ({aggregate.numeraire})
              </span>
              {total.deltaVector.length > 0 ? (
                <div className={styles.chips}>
                  {total.deltaVector.map((leg) => (
                    <span key={leg.ccy} className={styles.chip}>
                      <span className={styles.chipKey}>{leg.ccy}</span>
                      <span className={`num ${signClass(leg.amount)}`}>
                        {fmtPnlAdaptive(leg.amount)}
                      </span>
                    </span>
                  ))}
                </div>
              ) : (
                <span className={styles.emptyInline}>none</span>
              )}
            </div>
          </Panel>

          {/* --- limits RAG (celnet-limits) --- */}
          <Panel glyph="⚑" title="Limits" className={styles.limitsPanel}>
            <LimitsPanel limits={limits} />
          </Panel>
        </div>
      </div>
    </div>
  );
}

/** The limit utilization panel for the active scope, with honest empty-state. */
function LimitsPanel({ limits }: { limits: LimitStatusResponse }): React.ReactElement {
  if (limits.limits.length === 0) {
    return (
      <div className={styles.emptyInline}>
        No limits configured for this scope. Utilization and RAG appear here once a
        desk limit is set on the book.
      </div>
    );
  }
  return (
    <div className={styles.limits}>
      <div className={styles.limitsHead}>
        <span>
          worst:&nbsp;
          <span className={`${styles.rag} ${ragClass(limits.worst)}`}>{limits.worst}</span>
        </span>
        {limits.hardBreach && <span className={styles.hardBreach}>HARD BREACH</span>}
      </div>
      {limits.limits.map((u, i) => (
        <div key={`${u.metric}|${i}`} className={styles.limitRow}>
          <span className={styles.limitMetric}>{metricLabel(u)}</span>
          <span className={styles.limitBar}>
            <span
              className={`${styles.limitFill} ${ragClass(u.status)}`}
              style={{ width: `${Math.min(100, Math.max(0, u.ratio * 100))}%` }}
            />
          </span>
          <span className={`num ${styles.limitRatio} ${ragClass(u.status)}`}>
            {(u.ratio * 100).toFixed(0)}%
          </span>
          <span className={`${styles.ragDot} ${ragClass(u.status)}`} title={u.status} />
        </div>
      ))}
      <p className={styles.note}>
        Utilization = exposure ÷ cap, evaluated server-side by the limit framework.
        A HARD breach escalates; SOFT is advisory.
      </p>
    </div>
  );
}

// --- pure helpers -----------------------------------------------------------

/** Sum a set of node `AdditiveRisk` measures (the additive roll-up). */
function sumAdditive(parts: AdditiveRisk[]): AdditiveRisk {
  const deltaByCcy = new Map<string, number>();
  const acc: AdditiveRisk = {
    deltaNumeraire: 0,
    deltaVector: [],
    gamma: 0,
    vegaNumeraire: 0,
    theta: 0,
    vanna: 0,
    volga: 0,
    charm: 0,
    speed: 0,
    zomma: 0,
    color: 0,
    premiumNumeraire: 0,
    vegaLadder: [],
  };
  for (const p of parts) {
    acc.deltaNumeraire += p.deltaNumeraire;
    acc.gamma += p.gamma;
    acc.vegaNumeraire += p.vegaNumeraire;
    acc.theta += p.theta;
    acc.vanna += p.vanna;
    acc.volga += p.volga;
    acc.charm += p.charm;
    acc.speed += p.speed;
    acc.zomma += p.zomma;
    acc.color += p.color;
    acc.premiumNumeraire += p.premiumNumeraire;
    for (const leg of p.deltaVector) {
      deltaByCcy.set(leg.ccy, (deltaByCcy.get(leg.ccy) ?? 0) + leg.amount);
    }
  }
  acc.deltaVector = [...deltaByCcy.entries()]
    .map(([ccy, amount]) => ({ ccy, amount }))
    .sort((a, b) => Math.abs(b.amount) - Math.abs(a.amount));
  return acc;
}

/** Merge every node's vega ladder into one book-level ladder, keyed by pillar. */
function mergeVegaLadder(
  nodes: RiskNode[],
): { tenorDays: number; deltaBp: number; vega: number }[] {
  const byKey = new Map<string, { tenorDays: number; deltaBp: number; vega: number }>();
  for (const n of nodes) {
    for (const b of n.additive.vegaLadder) {
      const key = `${b.pillar.tenorDays}|${b.pillar.deltaBp}`;
      const cur = byKey.get(key);
      if (cur) cur.vega += b.vega;
      else byKey.set(key, { tenorDays: b.pillar.tenorDays, deltaBp: b.pillar.deltaBp, vega: b.vega });
    }
  }
  return [...byKey.values()].sort(
    (a, b) => a.tenorDays - b.tenorDays || Math.abs(b.vega) - Math.abs(a.vega),
  );
}

/** Reconstruct a priceable `Instrument` from a server `RiskPosition` (drill). */
function positionToInstrument(pos: RiskPosition): Instrument {
  const { inputs, org } = pos;
  // The server's canonical leaf is a single vanilla at an absolute strike; Risk
  // analyses exactly that leaf (the honest contributing position, not a synthetic
  // "net" instrument the contract cannot price).
  return {
    pair: org.ccyPair,
    tenor: { unit: "YEARS", count: Math.max(1, Math.round(inputs.t)) },
    expiryYears: inputs.t,
    quantity: { notional: Math.abs(pos.notionalBase), baseCcy: true },
    side: pos.notionalBase < 0 ? "SELL" : "BUY",
    product: {
      kind: "vanilla",
      vanilla: { optionType: pos.optionType, strike: { kind: "strike", strike: inputs.strike } },
    },
  };
}

/** A short option label for a drilled position. */
function optionShort(pos: RiskPosition): string {
  return pos.optionType === "CALL" ? "call" : "put";
}

/** Semantic sign class — bid/offer (green/red), never coral. Zero is neutral. */
function signClass(v: number): string {
  if (v > 0) return styles.pos ?? "";
  if (v < 0) return styles.neg ?? "";
  return "";
}

function ragClass(status: string): string {
  switch (status) {
    case "GREEN":
      return styles.ragGreen ?? "";
    case "AMBER":
      return styles.ragAmber ?? "";
    case "RED":
      return styles.ragRed ?? "";
    case "BREACH":
      return styles.ragBreach ?? "";
    default:
      return "";
  }
}

/** A human label for a node, by its dimension + group handle / dominant ccy. */
function nodeLabel(n: RiskNode): string {
  if (n.dimension === "CCY_PAIR") {
    const leg = n.additive.deltaVector[0];
    if (leg) return leg.ccy;
  }
  return `${dimensionHeader(n.dimension)} #${n.group}`;
}

function dimensionHeader(d: RiskDimension): string {
  switch (d) {
    case "FIRM":
      return "Firm";
    case "TRADER":
      return "Trader";
    case "BOOK":
      return "Book";
    case "DESK":
      return "Desk";
    case "CCY_PAIR":
      return "Currency";
    case "LOCATION":
      return "Location";
    case "ENTITY":
      return "Entity";
  }
}

function dimensionNoun(d: RiskDimension): string {
  switch (d) {
    case "CCY_PAIR":
      return "currencies";
    case "TRADER":
      return "traders";
    case "BOOK":
      return "books";
    case "DESK":
      return "desks";
    case "LOCATION":
      return "locations";
    case "ENTITY":
      return "entities";
    case "FIRM":
      return "firm";
  }
}

function pillarName(deltaBp: number): string {
  const abs = Math.abs(deltaBp);
  if (abs >= 4900) return "ATM";
  return `${deltaBp < 0 ? "−" : "+"}${Math.round(abs / 100)}Δ`;
}

/** A short metric label including the pillar/tenor payload where relevant. */
function metricLabel(u: LimitUtilization): string {
  switch (u.metric) {
    case "VEGA_BUCKET":
      return `Vega ${pillarName(u.vegaPillar.deltaBp)} ${tenorLabel(u.vegaPillar.tenorDays / 365)}`;
    case "TENOR_VEGA":
      return `Vega ${tenorLabel(u.tenorDays / 365)}`;
    case "CONCENTRATION_DELTA":
      return "Δ concentration";
    case "CONCENTRATION_VEGA":
      return "Vega concentration";
    case "EXPECTED_SHORTFALL":
      return "ES";
    case "STOP_LOSS":
      return "Stop-loss";
    default:
      return metricTitle(u.metric);
  }
}

function metricTitle(m: string): string {
  return m.charAt(0) + m.slice(1).toLowerCase();
}
