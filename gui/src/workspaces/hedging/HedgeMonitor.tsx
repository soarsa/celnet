/**
 * HedgeMonitor — the live hedge-ops dashboard (docs/AUTO-HEDGING §8.1). Rebuilt from a
 * flat dump into four answer-first sections a desk actually reads:
 *
 *   1. Engine status strip  — mode / kill-switch / per-desk enable, today's external
 *      hedged notional vs the daily cap (progress bar), max-clip, composite spread.
 *   2. Per-book RAG board   — one row per book: utilisation bar banded GREEN/AMBER/RED
 *      against its threshold, plus net risk — so near/over-cap books jump out.
 *   3. Live hedges          — the fired provenance with a running summary (count, total
 *      external notional, avg slippage, venue mix COMPOSITE vs LP), LIVE vs advisory.
 *   4. Needs attention      — unhedged breaches, escalations, and advisory intents that
 *      would trade if the engine were armed.
 *
 * Purely presentational — the container streams the intents + provenance and passes the
 * engine {@link HedgeConfig} for the status strip + cap gauge.
 */
import { useMemo } from "react";

import type { HedgeConfig, HedgeExecutionMode, HedgeIntent, HedgeProvenance } from "../../data/contract";
import { describeExitAction } from "../../lib/hedgeExit";
import styles from "./HedgingWorkspace.module.css";

interface HedgeMonitorProps {
  intents: readonly HedgeIntent[];
  provenance: readonly HedgeProvenance[];
  config: HedgeConfig | null;
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

const MODE_LABEL: Record<HedgeExecutionMode, string> = {
  advisory: "Advisory",
  lp_panel: "LP panel",
  composite: "Composite",
  lp_panel_then_composite: "LP panel → Composite",
};
function modeUsesComposite(m: HedgeExecutionMode): boolean {
  return m === "composite" || m === "lp_panel_then_composite";
}

/** The engine's overall armed state → tone + label for the status strip. */
function engineState(config: HedgeConfig): { tone: "red" | "amber" | "green"; label: string } {
  if (config.killSwitch) return { tone: "red", label: "KILL — halted" };
  if (config.execution === "advisory") return { tone: "amber", label: "Advisory (dry-run)" };
  return { tone: "green", label: "Armed — trading live" };
}

// --- section: engine status strip ------------------------------------------

function EngineStrip({
  config,
  externalToday,
}: {
  config: HedgeConfig;
  externalToday: number;
}): React.ReactElement {
  const state = engineState(config);
  const desksOn = config.deskEnabled.filter((d) => d.enabled).length;
  const cap = config.dailyExternalNotionalCap;
  const capPct = cap > 0 ? Math.min(1, externalToday / cap) : 0;
  const capTone = capPct >= 1 ? "red" : capPct >= 0.8 ? "amber" : "green";

  return (
    <div className={styles.engineStrip} data-testid="engine-strip">
      <div className={`${styles.engineState} ${styles[`engineState_${state.tone}`]}`} data-testid="engine-state">
        <span className={styles.engineDot} aria-hidden="true" />
        <span className={styles.engineStateLabel}>{state.label}</span>
      </div>
      <div className={styles.engineStats}>
        <div className={styles.engineStat}>
          <span className={styles.engineStatLabel}>Mode</span>
          <span className={styles.engineStatValue}>{MODE_LABEL[config.execution]}</span>
        </div>
        <div className={styles.engineStat}>
          <span className={styles.engineStatLabel}>Desks on</span>
          <span className={styles.engineStatValue}>
            {config.deskEnabled.length > 0 ? `${desksOn}/${config.deskEnabled.length}` : "all"}
          </span>
        </div>
        <div className={styles.engineStat}>
          <span className={styles.engineStatLabel}>Max clip</span>
          <span className={styles.engineStatValue}>{compact(config.maxClip)}</span>
        </div>
        {modeUsesComposite(config.execution) && (
          <div className={styles.engineStat}>
            <span className={styles.engineStatLabel}>Composite spread</span>
            <span className={styles.engineStatValue}>{config.compositeSpreadBp} bp</span>
          </div>
        )}
        <div className={`${styles.engineStat} ${styles.engineCap}`} data-testid="engine-cap">
          <span className={styles.engineStatLabel}>
            External today {cap > 0 ? `· cap ${compact(cap)}` : "· no cap"}
          </span>
          <span className={styles.engineStatValue}>
            {compact(externalToday)}
            {cap > 0 && <span className={styles.engineCapPct}> ({(capPct * 100).toFixed(0)}%)</span>}
          </span>
          {cap > 0 && (
            <span className={styles.capBar} aria-hidden="true">
              <span
                className={`${styles.capFill} ${styles[`capFill_${capTone}`]}`}
                style={{ width: `${capPct * 100}%` }}
              />
            </span>
          )}
        </div>
      </div>
    </div>
  );
}

// --- section: per-book RAG board -------------------------------------------

interface BookRag {
  book: string;
  band: string;
  utilization: number;
  netRisk: number;
}

function RagBoard({ books }: { books: readonly BookRag[] }): React.ReactElement {
  return (
    <section className={styles.subPanel} aria-labelledby="rag-board-heading" data-testid="rag-board">
      <h4 id="rag-board-heading" className={styles.formHeading}>
        Per-book risk
      </h4>
      {books.length === 0 ? (
        <p className={styles.emptyNote} data-testid="rag-empty">
          Waiting for the first risk-state tick…
        </p>
      ) : (
        <ul className={styles.ragBoardList}>
          {books.map((b) => {
            const key = ragKey(b.band);
            // Bar fills to utilisation, clamped at 120% so an over-cap book still reads.
            const pct = Math.min(1.2, Math.max(0, b.utilization)) / 1.2;
            return (
              <li key={b.book} className={styles.ragBoardRow} data-testid={`rag-book-${b.book}`}>
                <span className={styles.ragBoardBook}>{b.book}</span>
                <span className={styles.ragBoardBar} aria-hidden="true">
                  <span className={styles.ragBoardThreshold} style={{ left: `${(1 / 1.2) * 100}%` }} />
                  <span
                    className={`${styles.ragBoardFill} ${styles[`ragFill_${key}`]}`}
                    style={{ width: `${pct * 100}%` }}
                  />
                </span>
                <span className={`${styles.ragChip} ${styles[`rag_${key}`]}`}>{b.band.toUpperCase()}</span>
                <span className={styles.ragBoardUtil}>{(b.utilization * 100).toFixed(0)}%</span>
                <span className={styles.ragBoardNet}>net {compact(b.netRisk)}</span>
              </li>
            );
          })}
        </ul>
      )}
    </section>
  );
}

// --- section: live hedges ---------------------------------------------------

function venueOf(p: HedgeProvenance): "composite" | "lp" | "internal" {
  if (p.externalHedged <= 0) return "internal";
  return p.lpWon === "COMPOSITE" ? "composite" : "lp";
}

function LiveHedges({ provenance }: { provenance: readonly HedgeProvenance[] }): React.ReactElement {
  const summary = useMemo(() => {
    const live = provenance.filter((p) => !p.advisory);
    const external = live.filter((p) => p.externalHedged > 0);
    const totalExternal = external.reduce((s, p) => s + p.externalHedged, 0);
    const avgSlippage =
      external.length > 0 ? external.reduce((s, p) => s + Math.abs(p.slippageBp), 0) / external.length : 0;
    const composite = external.filter((p) => venueOf(p) === "composite").length;
    const lp = external.filter((p) => venueOf(p) === "lp").length;
    return {
      liveCount: live.length,
      advisoryCount: provenance.length - live.length,
      totalExternal,
      avgSlippage,
      composite,
      lp,
    };
  }, [provenance]);

  const rows = useMemo(
    () => [...provenance].sort((a, b) => b.firedAt - a.firedAt).slice(0, 14),
    [provenance],
  );

  return (
    <section className={styles.subPanel} aria-labelledby="live-hedges-heading" data-testid="live-hedges">
      <h4 id="live-hedges-heading" className={styles.formHeading}>
        Live hedges
      </h4>
      <div className={styles.sumRow} data-testid="live-summary">
        <span className={styles.sumChip}>
          <strong>{summary.liveCount}</strong> live
        </span>
        {summary.advisoryCount > 0 && (
          <span className={styles.sumChip}>
            <strong>{summary.advisoryCount}</strong> advisory
          </span>
        )}
        <span className={styles.sumChip}>
          ext <strong>{compact(summary.totalExternal)}</strong>
        </span>
        <span className={styles.sumChip}>
          avg slip <strong>{summary.avgSlippage.toFixed(2)}</strong> bp
        </span>
        <span className={styles.sumChip} data-testid="venue-mix">
          venue <strong>{summary.composite}</strong> comp · <strong>{summary.lp}</strong> LP
        </span>
      </div>

      {rows.length === 0 ? (
        <p className={styles.emptyNote} data-testid="provenance-empty">
          No fired hedges yet.
        </p>
      ) : (
        <div className={styles.tableScroll} tabIndex={0} role="group" aria-label="Fired hedges">
          <table className={styles.dataTable} data-testid="provenance-table">
            <thead>
              <tr>
                <th scope="col">When</th>
                <th scope="col">Book · instr</th>
                <th scope="col">Action</th>
                <th scope="col">External</th>
                <th scope="col">Venue</th>
                <th scope="col">Slip bp</th>
                <th scope="col">Mode</th>
              </tr>
            </thead>
            <tbody>
              {rows.map((p) => {
                const venue = venueOf(p);
                return (
                  <tr key={p.hedgeId} data-testid={`provenance-row-${p.hedgeId}`}>
                    <td>{timeOf(p.firedAt)}</td>
                    <td>
                      {p.book} · {p.instrument}
                    </td>
                    <td>{describeExitAction(p.action)}</td>
                    <td className={styles.num}>{p.externalHedged > 0 ? compact(p.externalHedged) : "—"}</td>
                    <td>
                      {venue === "internal" ? (
                        <span className={styles.lpMuted}>internal</span>
                      ) : venue === "composite" ? (
                        <span className={styles.venueComposite}>COMPOSITE</span>
                      ) : (
                        <span className={styles.venueLp}>{p.lpWon ?? "LP"}</span>
                      )}
                    </td>
                    <td className={styles.num}>{p.externalHedged > 0 ? p.slippageBp.toFixed(2) : "—"}</td>
                    <td>
                      {p.advisory ? (
                        <span className={styles.advisoryBadge}>ADVISORY</span>
                      ) : (
                        <span className={styles.liveBadge}>LIVE</span>
                      )}
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      )}
    </section>
  );
}

// --- section: needs attention ----------------------------------------------

interface AttnItem {
  book: string;
  tone: "red" | "amber";
  text: string;
}

function needsAttention(latestByBook: readonly HedgeIntent[]): AttnItem[] {
  const items: AttnItem[] = [];
  for (const i of latestByBook) {
    const key = ragKey(i.band);
    const hedged = i.internalCrossed + i.externalHedged;
    if ((key === "red" || key === "breach") && hedged <= 0) {
      items.push({
        book: i.book,
        tone: "red",
        text: `${i.band.toUpperCase()} on ${i.instrument} — nothing hedged (util ${(i.utilization * 100).toFixed(0)}%)`,
      });
    } else if (i.action?.kind === "escalate") {
      items.push({ book: i.book, tone: "amber", text: `Escalated to desk on ${i.instrument}` });
    } else if (i.advisory && i.externalHedged > 0) {
      items.push({
        book: i.book,
        tone: "amber",
        text: `Advisory — would externalise ${compact(i.externalHedged)} on ${i.instrument} if armed`,
      });
    }
  }
  return items;
}

function NeedsAttention({ items }: { items: readonly AttnItem[] }): React.ReactElement {
  return (
    <section className={styles.subPanel} aria-labelledby="attn-heading" data-testid="needs-attention">
      <h4 id="attn-heading" className={styles.formHeading}>
        Needs attention {items.length > 0 && <span className={styles.attnCount}>{items.length}</span>}
      </h4>
      {items.length === 0 ? (
        <p className={styles.emptyNote} data-testid="attn-clear">
          <span aria-hidden="true">✓</span> Nothing needs attention — no unhedged breaches or escalations.
        </p>
      ) : (
        <ul className={styles.attnList}>
          {items.map((a, idx) => (
            <li key={`${a.book}-${idx}`} className={styles.attnRow} data-testid="attn-row">
              <span className={`${styles.attnDot} ${styles[`attnDot_${a.tone}`]}`} aria-hidden="true" />
              <span className={styles.attnBook}>{a.book}</span>
              <span className={styles.attnText}>{a.text}</span>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}

// --- host ------------------------------------------------------------------

export function HedgeMonitor({ intents, provenance, config }: HedgeMonitorProps): React.ReactElement {
  // Latest intent per book (newest wins) → the RAG board + attention derivations.
  const latestByBook = useMemo(() => {
    const m = new Map<string, HedgeIntent>();
    for (const i of intents) m.set(i.book, i);
    return [...m.values()].sort((a, b) => a.book.localeCompare(b.book));
  }, [intents]);

  const rag: BookRag[] = useMemo(
    () => latestByBook.map((i) => ({ book: i.book, band: i.band, utilization: i.utilization, netRisk: i.netRisk })),
    [latestByBook],
  );

  const externalToday = useMemo(
    () => provenance.filter((p) => !p.advisory).reduce((s, p) => s + p.externalHedged, 0),
    [provenance],
  );

  const attn = useMemo(() => needsAttention(latestByBook), [latestByBook]);

  return (
    <section className={styles.monitor} aria-labelledby="hedge-monitor-heading" data-testid="hedge-monitor">
      <h3 id="hedge-monitor-heading" className={styles.panelHeading}>
        Hedge monitor
      </h3>

      {config !== null && <EngineStrip config={config} externalToday={externalToday} />}

      <div className={styles.monitorGrid}>
        <RagBoard books={rag} />
        <NeedsAttention items={attn} />
      </div>

      <LiveHedges provenance={provenance} />
    </section>
  );
}
