/**
 * HedgeMonitor — the live hedge surface (docs/AUTO-HEDGING §8.1): a per-book RAG chip
 * strip, the stream of advisory hedge INTENTS (each with an ADVISORY badge when the
 * policy is armed dry-run), and the immutable fired-{@link HedgeProvenance} audit rows.
 * Purely presentational — the container feeds it the streamed intents + provenance.
 */
import { useMemo } from "react";

import type { HedgeIntent, HedgeProvenance } from "../../data/contract";
import { describeExitAction } from "../../lib/hedgeExit";
import styles from "./HedgingWorkspace.module.css";

interface HedgeMonitorProps {
  intents: readonly HedgeIntent[];
  provenance: readonly HedgeProvenance[];
}

/** Normalise a band label to a RAG class key. */
function ragKey(band: string): "green" | "amber" | "red" | "breach" {
  if (band === "amber") return "amber";
  if (band === "red") return "red";
  if (band === "breach") return "breach";
  return "green";
}

function compact(n: number): string {
  return new Intl.NumberFormat("en-US", { notation: "compact", maximumFractionDigits: 1 }).format(n);
}

function timeOf(ms: number): string {
  return new Date(ms).toLocaleTimeString("en-GB", { hour12: false });
}

export function HedgeMonitor({ intents, provenance }: HedgeMonitorProps): React.ReactElement {
  // Latest band per book (newest intent wins) → the RAG chip strip.
  const bookBands = useMemo(() => {
    const m = new Map<string, { band: string; utilization: number }>();
    for (const i of intents) m.set(i.book, { band: i.band, utilization: i.utilization });
    return [...m.entries()].sort((a, b) => a[0].localeCompare(b[0]));
  }, [intents]);

  const recentIntents = useMemo(() => [...intents].slice(-12).reverse(), [intents]);

  return (
    <section className={styles.monitor} aria-labelledby="hedge-monitor-heading" data-testid="hedge-monitor">
      <h3 id="hedge-monitor-heading" className={styles.panelHeading}>
        Hedge monitor
      </h3>

      <div className={styles.ragStrip} aria-label="Per-book risk band" data-testid="rag-strip">
        {bookBands.length === 0 ? (
          <span className={styles.emptyNote} data-testid="rag-empty">
            Waiting for the first risk-state tick…
          </span>
        ) : (
          bookBands.map(([book, { band, utilization }]) => (
            <div key={book} className={styles.ragCard} data-testid={`rag-book-${book}`}>
              <span className={`${styles.ragChip} ${styles[`rag_${ragKey(band)}`]}`}>
                {band.toUpperCase()}
              </span>
              <span className={styles.ragBook}>{book}</span>
              <span className={styles.ragUtil}>{(utilization * 100).toFixed(0)}%</span>
            </div>
          ))
        )}
      </div>

      <div className={styles.monitorCols}>
        <div>
          <h4 className={styles.formHeading}>Advisory intents (live)</h4>
          {recentIntents.length === 0 ? (
            <p className={styles.emptyNote} data-testid="intents-empty">
              No intents yet.
            </p>
          ) : (
            <ul
              className={styles.intentList}
              data-testid="intent-list"
              tabIndex={0}
              aria-label="Advisory hedge intents (live)"
            >
              {recentIntents.map((i, idx) => (
                <li key={`${i.firedAt}-${idx}`} className={styles.intentRow} data-testid="intent-row">
                  <span className={`${styles.ragChip} ${styles[`rag_${ragKey(i.band)}`]}`}>
                    {i.band.toUpperCase()}
                  </span>
                  <span className={styles.intentBook}>
                    {i.book} · {i.instrument}
                  </span>
                  <span className={styles.intentAction}>{describeExitAction(i.action)}</span>
                  <span className={styles.intentNums}>
                    util {(i.utilization * 100).toFixed(0)}% · overflow {compact(i.overflow)}
                  </span>
                  {i.advisory && (
                    <span className={styles.advisoryBadge} data-testid="advisory-badge">
                      ADVISORY
                    </span>
                  )}
                  <span className={styles.intentTime}>{timeOf(i.firedAt)}</span>
                </li>
              ))}
            </ul>
          )}
        </div>

        <div>
          <h4 className={styles.formHeading}>Fired provenance (audit)</h4>
          {provenance.length === 0 ? (
            <p className={styles.emptyNote} data-testid="provenance-empty">
              No fired hedges yet.
            </p>
          ) : (
            <div
              className={styles.tableScroll}
              tabIndex={0}
              role="group"
              aria-label="Fired hedge provenance (audit)"
            >
              <table className={styles.dataTable} data-testid="provenance-table">
                <thead>
                  <tr>
                    <th scope="col">When</th>
                    <th scope="col">Book · instr</th>
                    <th scope="col">Band</th>
                    <th scope="col">Action</th>
                    <th scope="col">Internal</th>
                    <th scope="col">External</th>
                    <th scope="col">LP</th>
                    <th scope="col">Mode</th>
                  </tr>
                </thead>
                <tbody>
                  {[...provenance]
                    .sort((a, b) => b.firedAt - a.firedAt)
                    .slice(0, 20)
                    .map((p) => (
                      <tr key={p.hedgeId} data-testid={`provenance-row-${p.hedgeId}`}>
                        <td>{timeOf(p.firedAt)}</td>
                        <td>
                          {p.book} · {p.instrument}
                        </td>
                        <td>
                          <span className={`${styles.ragChip} ${styles[`rag_${ragKey(p.band)}`]}`}>
                            {p.band.toUpperCase()}
                          </span>
                        </td>
                        <td>{describeExitAction(p.action)}</td>
                        <td className={styles.num}>{compact(p.internalCrossed)}</td>
                        <td className={styles.num}>{compact(p.externalHedged)}</td>
                        <td>{p.lpWon ?? "—"}</td>
                        <td>
                          {p.advisory ? (
                            <span className={styles.advisoryBadge}>ADVISORY</span>
                          ) : (
                            <span className={styles.liveBadge}>LIVE</span>
                          )}
                        </td>
                      </tr>
                    ))}
                </tbody>
              </table>
            </div>
          )}
        </div>
      </div>
    </section>
  );
}
