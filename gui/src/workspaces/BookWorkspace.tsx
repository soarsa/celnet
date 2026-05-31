/**
 * BookWorkspace — the DESK-WIDE AGGREGATED risk view (closes the product owner's
 * gap "how can a user see aggregated risk?"). Where the Risk workspace analyses a
 * single structure, this sums the REAL per-position risk across ALL open positions
 * across ALL pairs (`app.positions`) into one book-level picture:
 *
 *   - summary cards: net P&L mark, net vega, net gamma, net theta;
 *   - a per-pair breakdown table: net delta / vega / gamma / theta + notional;
 *   - an aggregate vega ladder bucketed by (tenor × delta);
 *   - aggregate cross-gamma + theta-roll disclosures.
 *
 * Every number is genuine: each position is repriced via SurfaceService.Scenario
 * at its own pair's base market — once for base Greeks, once with a
 * RiskBucketRequest for the book-shaped decomposition — scaled by the position's
 * *signed* notional (long +, short −) and summed (see data/portfolioRisk.ts).
 * Nothing is fabricated; if there are no open positions an honest empty-state is
 * shown, and positions whose pair has no known market are disclosed, not dropped.
 */

import { useEffect, useMemo, useState } from "react";
import { useApp } from "../app/AppContext";
import type { Instrument, MarketContext } from "../data/contract";
import {
  aggregateBookRisk,
  resolveBookPositions,
  type BookRisk,
} from "../data/portfolioRisk";
import { Panel } from "../components/Panel";
import { fmtPnlAdaptive, fmtSigned } from "../lib/format";
import { tenorLabel } from "../lib/trend";
import styles from "./BookWorkspace.module.css";

/** A pair's drill target: a representative open position and how it is labelled. */
interface PairDrill {
  /** The largest |notional| contributing position for the pair. */
  instrument: Instrument;
  /** Honest label: the contributor's structure + how many positions it stands for. */
  label: string;
  /** Count of open positions in this pair (so the label can disclose the cohort). */
  count: number;
}

export function BookWorkspace(): React.ReactElement {
  const app = useApp();
  const [book, setBook] = useState<BookRisk | null>(null);

  // Resolve each position to its pair's base market (the watched-pairs context).
  // Positions whose pair has no known market are split out and disclosed honestly
  // rather than priced against a fabricated market.
  const { resolved, skipped } = useMemo(() => {
    const marketByPair = new Map<string, MarketContext>();
    for (const p of app.pairs) {
      marketByPair.set(`${p.pair.base}/${p.pair.quote}`, p.market);
    }
    return resolveBookPositions(app.positions, marketByPair);
  }, [app.positions, app.pairs]);

  useEffect(() => {
    let live = true;
    void aggregateBookRisk(resolved, app.conventions, app.transport, skipped).then((b) => {
      if (live) setBook(b);
    });
    return () => {
      live = false;
    };
  }, [resolved, skipped, app.conventions, app.transport]);

  // Per-pair drill target: Book and Risk are the same cube at two zooms — clicking
  // a breakdown row opens the largest contributing position of that pair in the
  // Risk workspace. A single pair can hold several positions, so we drill into the
  // largest |notional| contributor and label it honestly ("of N") rather than
  // fabricating one synthetic "net" instrument that the contract cannot price.
  const drillByPair = useMemo(() => {
    const byPair = new Map<string, PairDrill>();
    for (const { instrument } of resolved) {
      const key = `${instrument.pair.base}/${instrument.pair.quote}`;
      const cur = byPair.get(key);
      if (!cur) {
        byPair.set(key, {
          instrument,
          label: structureLabel(instrument),
          count: 1,
        });
      } else {
        cur.count += 1;
        if (instrument.quantity.notional > cur.instrument.quantity.notional) {
          cur.instrument = instrument;
          cur.label = structureLabel(instrument);
        }
      }
    }
    return byPair;
  }, [resolved]);

  const drillPair = (pairKey: string): void => {
    const d = drillByPair.get(pairKey);
    if (!d) return;
    const cohort = d.count > 1 ? ` (largest of ${d.count})` : "";
    app.drillToRisk(d.instrument, `${pairKey} ${d.label}${cohort}`);
  };

  if (book === null) {
    return <div className={styles.loading}>Aggregating book risk…</div>;
  }

  if (book.positionCount === 0) {
    return (
      <div className={styles.empty}>
        <span className={styles.emptyGlyph}>Σ</span>
        <p className={styles.emptyTitle}>No open positions</p>
        <p className={styles.emptyBody}>
          The desk book is empty. Stream a structure into the blotter or book a
          ticket; desk-wide aggregated risk appears here the moment a position
          exists. Nothing is shown until there is something real to aggregate.
        </p>
        {skipped.length > 0 && (
          <p className={styles.emptyBody}>
            {skipped.length} position{skipped.length === 1 ? "" : "s"} could not be
            priced (no marked market for their pair) and were excluded.
          </p>
        )}
      </div>
    );
  }

  const { total } = book;

  const cards = [
    { label: "Net P&L", value: total.price, kind: "pnl" as const },
    { label: "Net Vega", value: total.vega, kind: "pnl" as const },
    { label: "Net Gamma", value: total.gamma, kind: "pnl" as const },
    { label: "Net Theta", value: total.theta, kind: "pnl" as const },
  ];

  const maxVega = Math.max(...book.vegaLadder.map((b) => Math.abs(b.vegaPerPoint)), 1e-12);

  return (
    <div className={styles.wrap}>
      {/* --- book-total summary cards --- */}
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
        {/* --- per-pair breakdown --- */}
        <Panel
          glyph="Σ"
          title={`Book breakdown · ${book.positionCount} positions · ${book.pairCount} pairs`}
          className={styles.tablePanel}
        >
          <table className={styles.table}>
            <thead>
              <tr>
                <th className={styles.colPair}>Pair</th>
                <th className={`num ${styles.colNum}`}>Pos</th>
                <th className={`num ${styles.colNum}`}>Notional</th>
                <th className={`num ${styles.colNum}`}>Net Δ</th>
                <th className={`num ${styles.colNum}`}>Net Vega</th>
                <th className={`num ${styles.colNum}`}>Net Gamma</th>
                <th className={`num ${styles.colNum}`}>Net Theta</th>
              </tr>
            </thead>
            <tbody>
              {book.byPair.map((p) => {
                const drill = drillByPair.get(p.pairKey);
                return (
                <tr
                  key={p.pairKey}
                  className={drill ? styles.drillRow : ""}
                  onClick={drill ? () => drillPair(p.pairKey) : undefined}
                  role={drill ? "button" : undefined}
                  tabIndex={drill ? 0 : undefined}
                  title={
                    drill
                      ? `Drill to Risk · ${drill.label}${drill.count > 1 ? ` (largest of ${drill.count})` : ""}`
                      : undefined
                  }
                  onKeyDown={
                    drill
                      ? (e) => {
                          if (e.key === "Enter" || e.key === " ") {
                            e.preventDefault();
                            drillPair(p.pairKey);
                          }
                        }
                      : undefined
                  }
                >
                  <td className={styles.pairCell}>
                    {drill && <span className={styles.drillCue} aria-hidden>›</span>}
                    {p.base}/{p.quote}
                  </td>
                  <td className="num">{p.count}</td>
                  <td className={`num ${styles.notional}`}>{fmtPnlAdaptive(p.netNotional)}</td>
                  <td className={`num ${signClass(p.netDelta)}`}>{fmtPnlAdaptive(p.netDelta)}</td>
                  <td className={`num ${signClass(p.netVega)}`}>{fmtPnlAdaptive(p.netVega)}</td>
                  <td className={`num ${signClass(p.netGamma)}`}>{fmtPnlAdaptive(p.netGamma)}</td>
                  <td className={`num ${signClass(p.netTheta)}`}>{fmtPnlAdaptive(p.netTheta)}</td>
                </tr>
                );
              })}
            </tbody>
            <tfoot>
              <tr>
                <td className={styles.pairCell}>All pairs</td>
                <td className="num">{book.positionCount}</td>
                <td className={`num ${styles.notional}`}>{fmtPnlAdaptive(book.grossNotional)}</td>
                <td className={`num ${signClass(total.deltaSpot)}`}>{fmtPnlAdaptive(total.deltaSpot)}</td>
                <td className={`num ${signClass(total.vega)}`}>{fmtPnlAdaptive(total.vega)}</td>
                <td className={`num ${signClass(total.gamma)}`}>{fmtPnlAdaptive(total.gamma)}</td>
                <td className={`num ${signClass(total.theta)}`}>{fmtPnlAdaptive(total.theta)}</td>
              </tr>
            </tfoot>
          </table>
          <p className={styles.note}>
            Click a pair row to drill into its largest position in Risk — Book and
            Risk are the same cube at two zooms.{" "}
            Net Greeks are summed from each position repriced via SurfaceService.Scenario
            at its pair&apos;s base market, scaled by signed notional (long +, short −).
            Cross-pair totals are summed in each pair&apos;s <em>native premium units</em> —
            not yet converted to a common reporting numeraire, so a high-spot pair (e.g.
            USD/JPY) dominates a raw sum. Common-numeraire delta/vega normalisation and
            cross-pair delta triangulation are tracked in the risk-hierarchy workstream
            (see docs/RISK-HIERARCHY.md).
            {skipped.length > 0 && (
              <>
                {" "}
                {skipped.length} position{skipped.length === 1 ? "" : "s"} excluded — no
                marked market for their pair.
              </>
            )}
          </p>
        </Panel>

        {/* --- aggregate vega ladder --- */}
        <Panel glyph="ν" title="Aggregate vega ladder" className={styles.ladderPanel}>
          {book.vegaLadder.length > 0 && book.vegaLadder.some((b) => Math.abs(b.vegaPerPoint) > 0) ? (
            <div className={styles.ladder}>
              {/* "Σ Vega" disambiguates this as the book SUM across positions (the
                  Σ glyph means summed-across-the-book here, never a single line); the
                  caveat below keeps the honest native-premium-units numeraire note. */}
              <div className={styles.ladderHead}>
                <span>Tenor</span>
                <span>Pillar</span>
                <span>Σ Vega / vol-pt</span>
              </div>
              {book.vegaLadder.map((b) => (
                <div key={`${b.tenorYears}|${b.delta}`} className={styles.ladderRow}>
                  <span className="num">{tenorLabel(b.tenorYears)}</span>
                  <span className="num">{pillarName(b.delta)}</span>
                  <span className={styles.bar}>
                    <span
                      className={styles.barFill}
                      style={{
                        width: `${(Math.abs(b.vegaPerPoint) / maxVega) * 100}%`,
                        background:
                          b.vegaPerPoint >= 0
                            ? "oklch(from var(--bid) l c h / 0.4)"
                            : "oklch(from var(--offer) l c h / 0.4)",
                      }}
                    />
                    <span className={`num ${styles.barVal} ${signClass(b.vegaPerPoint)}`}>
                      {fmtPnlAdaptive(b.vegaPerPoint)}
                    </span>
                  </span>
                </div>
              ))}
            </div>
          ) : (
            <div className={styles.emptyInline}>
              No vega at the standard pillars across the book.
            </div>
          )}

          <p className={styles.ladderNote}>
            Σ = summed across the book&apos;s positions, in each pair&apos;s{" "}
            <em>native premium units</em> — not yet a common reporting numeraire.
          </p>

          <div className={styles.disclosure}>
            <span className={`brand-label ${styles.discLabel}`}>cross-gamma</span>
            {book.crossGammas.length > 0 ? (
              <div className={styles.chips}>
                {book.crossGammas.map((cg) => (
                  <span key={`${cg.factorA}|${cg.factorB}`} className={styles.chip}>
                    <span className={styles.chipKey}>
                      {factorShort(cg.factorA)}×{factorShort(cg.factorB)}
                    </span>
                    <span className={`num ${signClass(cg.value)}`}>{fmtSigned(cg.value, 2)}</span>
                  </span>
                ))}
              </div>
            ) : (
              <span className={styles.emptyInline}>none</span>
            )}
          </div>

          <div className={styles.disclosure}>
            <span className={`brand-label ${styles.discLabel}`}>theta roll</span>
            {book.thetaRoll.length > 0 ? (
              <div className={styles.chips}>
                {book.thetaRoll.map((t) => (
                  <span key={t.horizonYears} className={styles.chip}>
                    <span className={styles.chipKey}>{tenorLabel(t.horizonYears)}</span>
                    <span className={`num ${signClass(t.pnl)}`}>{fmtPnlAdaptive(t.pnl)}</span>
                  </span>
                ))}
              </div>
            ) : (
              <span className={styles.emptyInline}>none</span>
            )}
          </div>
        </Panel>
      </div>
    </div>
  );
}

/** Semantic sign class — bid/offer (green/red), never coral. Zero is neutral. */
function signClass(v: number): string {
  if (v > 0) return styles.pos ?? "";
  if (v < 0) return styles.neg ?? "";
  return "";
}

/**
 * An honest short label for an instrument's structure, used when drilling a pair
 * row into Risk: the tenor (via the shared `tenorLabel`) + the product shape
 * (strategy kind or vanilla call/put). Notional is implied by the position itself.
 */
function structureLabel(instrument: Instrument): string {
  const t = tenorLabel(instrument.expiryYears);
  if (instrument.product.kind === "strategy") {
    return `${t} ${strategyShort(instrument.product.strategy.kind)}`;
  }
  return `${t} ${instrument.product.vanilla.optionType === "CALL" ? "call" : "put"}`;
}

function strategyShort(kind: string): string {
  switch (kind) {
    case "RISK_REVERSAL":
      return "RR";
    case "STRANGLE":
      return "strangle";
    case "STRADDLE":
      return "straddle";
    case "SEAGULL":
      return "seagull";
    default:
      return kind;
  }
}

function pillarName(delta: number): string {
  if (Math.abs(delta) >= 0.49) return "ATM";
  return `${delta < 0 ? "−" : "+"}${Math.round(Math.abs(delta) * 100)}Δ`;
}

function factorShort(f: string): string {
  switch (f) {
    case "SPOT":
      return "S";
    case "VOL":
      return "σ";
    case "RATE_DOM":
      return "rd";
    case "RATE_FOR":
      return "rf";
    case "TIME":
      return "t";
    default:
      return f;
  }
}
