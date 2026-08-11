/**
 * HedgeMonitor — the live hedge-ops dashboard (docs/AUTO-HEDGING §8.1). SELF-CONTAINED:
 * it fetches its own engine {@link HedgeConfig}, subscribes to the live advisory-intent
 * stream, and refetches the fired provenance on each tick — so it drops into the Risk
 * surface's "Hedge flows" tab with NO parent state (the same data the Hedging Rules
 * surface used to thread in from its removed Monitor tab). The presentational body
 * ({@link HedgeMonitorView}) is unchanged. Rebuilt from a flat dump into five
 * answer-first sections a desk actually reads:
 *
 *   1. Engine status strip  — mode / kill-switch / per-desk enable, today's external
 *      hedged notional vs the daily cap (progress bar), max-clip, composite spread.
 *   2. Suggestions          — the STANDING rows raised by the `suggest` exit mode: a
 *      sized-but-untraded hedge per breached book, with "Hedge now" / "Dismiss" and NO
 *      confirmation dialog. Placed first because it is the only section that asks the
 *      desk to act. See {@link SuggestionsPanel}.
 *   3. Per-book RAG board   — one row per book: utilisation bar banded GREEN/AMBER/RED
 *      against its threshold, plus net risk — so near/over-cap books jump out.
 *   4. Needs attention      — unhedged breaches, escalations, and advisory intents that
 *      would trade if the engine were armed.
 *   5. Live hedges          — the fired provenance with a running summary (count, total
 *      external notional, avg slippage, venue mix COMPOSITE vs LP), LIVE vs advisory.
 *
 * The container owns the data: it loads the engine {@link HedgeConfig}, subscribes to the
 * live `hedge_intent` push, and refetches BOTH the fired provenance and the standing
 * suggestions on every tick (a `suggest`-scoped breach raises a row at exactly the moment
 * an `auto`-scoped one would have fired).
 */
import { useCallback, useEffect, useMemo, useState } from "react";

import { useApp } from "../../app/AppContext";
import { HelpButton } from "../../components/HelpButton";
import type {
  HedgeConfig,
  HedgeExecutionMode,
  HedgeIntent,
  HedgeProvenance,
  HedgeSuggestion,
  HedgeVehiclePlan,
} from "../../data/contract";
import { describeExitAction } from "../../lib/hedgeExit";
import {
  DURATION_PROXY_WARNING,
  formatDv01,
  isDurationProxy,
  residualWording,
} from "../../lib/hedgeVehicle";
import styles from "./HedgingWorkspace.module.css";

interface HedgeMonitorProps {
  intents: readonly HedgeIntent[];
  provenance: readonly HedgeProvenance[];
  config: HedgeConfig | null;
  /** The STANDING suggestions raised by the `suggest` exit mode. */
  suggestions: readonly HedgeSuggestion[];
  /** Suggestion ids with an in-flight execute/dismiss (their buttons are disabled). */
  busySuggestions: ReadonlySet<string>;
  /** A failed execute/dismiss, surfaced inline beside the restored row. */
  suggestionError: string | null;
  /** Hide the act buttons for a viewer without the `hedge` capability. */
  readOnly: boolean;
  /** Fire (`dismiss` false) or drop (`dismiss` true) one suggestion. NEVER confirms. */
  onActOnSuggestion: (id: string, dismiss: boolean) => void;
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

// --- section: standing suggestions (the suggest-then-exit surface) ----------

/** Units for display: whole lots print as integers, fractional face to 2dp. */
function unitsText(value: number, whole: boolean): string {
  return whole
    ? new Intl.NumberFormat("en-US", { maximumFractionDigits: 0 }).format(value)
    : new Intl.NumberFormat("en-US", { maximumFractionDigits: 2 }).format(value);
}

/**
 * The vehicle plan, rendered HONESTLY: what was asked for, what will actually trade, and
 * what that rounding leaves behind — with the sign of the residual spelled out in words,
 * because a bare signed number next to a size invites exactly the wrong reading.
 *
 * When the size came off a duration-BLIND exposure proxy the row leads with a warning.
 * That is not decoration: the proxy treats every bond as duration 1, so sizing a
 * DIFFERENT instrument off it is approximate by construction, and a desk that reads it as
 * exact will systematically mis-hedge. A proxy-based size is never presented as exact.
 */
function VehiclePlanLine({ plan }: { plan: HedgeVehiclePlan }): React.ReactElement {
  return (
    <div className={styles.planLine} data-testid="suggestion-plan">
      {isDurationProxy(plan) && (
        <span className={styles.proxyWarning} data-testid="suggestion-proxy-warning">
          <span aria-hidden="true">⚠</span> {DURATION_PROXY_WARNING}
        </span>
      )}
      <span className={styles.planFacts}>
        <span className={styles.planFact}>
          <span className={styles.planFactLabel}>Trades</span>
          <strong>
            {unitsText(plan.units, plan.wholeUnits)} {plan.unitLabel}
            {plan.units === 1 ? "" : "s"}
          </strong>
        </span>
        <span className={styles.planFact}>
          <span className={styles.planFactLabel}>Exact</span>
          {unitsText(plan.exactUnits, false)}
        </span>
        <span className={styles.planFact}>
          <span className={styles.planFactLabel}>Target DV01</span>
          {formatDv01(plan.targetDv01)}
        </span>
        <span className={styles.planFact}>
          <span className={styles.planFactLabel}>Hedged DV01</span>
          {formatDv01(plan.hedgedDv01)}
        </span>
        <span className={styles.planFact} data-testid="suggestion-residual">
          <span className={styles.planFactLabel}>Residual</span>
          {residualWording(plan)}
        </span>
      </span>
    </div>
  );
}

function SuggestionRow({
  suggestion,
  busy,
  readOnly,
  onAct,
}: {
  suggestion: HedgeSuggestion;
  busy: boolean;
  readOnly: boolean;
  onAct: (id: string, dismiss: boolean) => void;
}): React.ReactElement {
  const key = ragKey(suggestion.band);
  return (
    <li
      className={styles.suggestionRow}
      data-testid={`suggestion-${suggestion.suggestionId}`}
    >
      <div className={styles.suggestionMain}>
        <p className={styles.suggestionHeadline} data-testid="suggestion-headline">
          {suggestion.headline}
        </p>
        <p className={styles.suggestionContext}>
          <span className={`${styles.ragChip} ${styles[`rag_${key}`]}`}>
            {suggestion.band.toUpperCase()}
          </span>
          <span className={styles.suggestionBook}>
            {suggestion.book} · {suggestion.instrument}
          </span>
          <span className={styles.suggestionUtil}>
            {(suggestion.utilization * 100).toFixed(0)}% of budget
          </span>
          <span className={styles.suggestionWhen}>{timeOf(suggestion.raisedAt)}</span>
        </p>
        <p className={styles.suggestionRationale}>{suggestion.rationale}</p>
        {suggestion.vehiclePlan !== null && <VehiclePlanLine plan={suggestion.vehiclePlan} />}
      </div>
      {!readOnly && (
        <div className={styles.suggestionActions}>
          {/*
            No confirm dialog on EITHER button — that is the entire point of the feature.
            The standing row IS the deliberation step, so re-asking would reintroduce the
            modal the mode exists to avoid.
          */}
          <button
            type="button"
            className={styles.saveBtn}
            disabled={busy}
            data-testid={`suggestion-hedge-${suggestion.suggestionId}`}
            onClick={() => onAct(suggestion.suggestionId, false)}
          >
            {busy ? "Working…" : "Hedge now"}
          </button>
          <button
            type="button"
            className={styles.ghostBtn}
            disabled={busy}
            data-testid={`suggestion-dismiss-${suggestion.suggestionId}`}
            onClick={() => onAct(suggestion.suggestionId, true)}
          >
            Dismiss
          </button>
        </div>
      )}
    </li>
  );
}

/**
 * The standing-suggestion board — the `suggest` exit mode's surface.
 *
 * `aria-live="polite"` on the list so a newly raised suggestion is ANNOUNCED rather than
 * appearing silently: the whole premise is that nothing interrupts the trader, which makes
 * a screen-reader announcement the only signal a non-sighted user would otherwise get.
 */
function SuggestionsPanel({
  suggestions,
  busyIds,
  readOnly,
  error,
  onAct,
}: {
  suggestions: readonly HedgeSuggestion[];
  busyIds: ReadonlySet<string>;
  readOnly: boolean;
  error: string | null;
  onAct: (id: string, dismiss: boolean) => void;
}): React.ReactElement {
  const sorted = useMemo(
    () => [...suggestions].sort((a, b) => b.raisedAt - a.raisedAt),
    [suggestions],
  );

  return (
    <section
      className={styles.subPanel}
      aria-labelledby="suggestions-heading"
      data-testid="hedge-suggestions"
    >
      <h4 id="suggestions-heading" className={styles.formHeading}>
        Suggestions{" "}
        {sorted.length > 0 && <span className={styles.attnCount}>{sorted.length}</span>}{" "}
        <HelpButton helpId="concept.hedge-suggestion" subject="how to read a hedge suggestion" />
      </h4>
      <p className={styles.panelNote}>
        Books on the <strong>Suggest</strong> exit mode do not trade on a breach — the engine
        sizes the hedge and leaves it here. These rows stand until you act on them.
      </p>
      {error !== null && (
        <p className={styles.errorText} role="alert" data-testid="suggestion-error">
          {error}
        </p>
      )}
      <ul className={styles.suggestionList} aria-live="polite" data-testid="suggestion-list">
        {sorted.length === 0 ? (
          <li className={styles.emptyNote} data-testid="suggestions-empty">
            No standing suggestions.
          </li>
        ) : (
          sorted.map((s) => (
            <SuggestionRow
              key={s.suggestionId}
              suggestion={s}
              busy={busyIds.has(s.suggestionId)}
              readOnly={readOnly}
              onAct={onAct}
            />
          ))
        )}
      </ul>
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

// --- presentational view ----------------------------------------------------

function HedgeMonitorView({
  intents,
  provenance,
  config,
  suggestions,
  busySuggestions,
  suggestionError,
  readOnly,
  onActOnSuggestion,
}: HedgeMonitorProps): React.ReactElement {
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

      {/*
        The suggestions board sits ABOVE the read-only boards: it is the only section that
        asks the desk to DO something, and burying an actionable row under two dashboards
        is how a standing suggestion quietly becomes a stale one.
      */}
      <SuggestionsPanel
        suggestions={suggestions}
        busyIds={busySuggestions}
        readOnly={readOnly}
        error={suggestionError}
        onAct={onActOnSuggestion}
      />

      <div className={styles.monitorGrid}>
        <RagBoard books={rag} />
        <NeedsAttention items={attn} />
      </div>

      <LiveHedges provenance={provenance} />
    </section>
  );
}

// --- self-fetching container ------------------------------------------------

/**
 * HedgeMonitor — the SELF-CONTAINED live monitor mounted standalone in the Risk
 * surface's "Hedge flows" tab. Owns its own data exactly as the removed Hedging Rules
 * "Monitor" tab did: it loads the engine config once, subscribes to the live
 * advisory-intent stream (retaining the last 40), and refetches the fired provenance on
 * each tick so the audit rows track fires without a poll of its own. Purely a data
 * shell around {@link HedgeMonitorView}; it carries NO execution-mode config (that now
 * lives on Hedging Rules → Execution mode).
 */
export function HedgeMonitor(): React.ReactElement {
  const app = useApp();
  const [intents, setIntents] = useState<HedgeIntent[]>([]);
  const [provenance, setProvenance] = useState<HedgeProvenance[]>([]);
  const [config, setConfig] = useState<HedgeConfig | null>(null);
  const [suggestions, setSuggestions] = useState<HedgeSuggestion[]>([]);
  const [busySuggestions, setBusySuggestions] = useState<ReadonlySet<string>>(new Set());
  const [suggestionError, setSuggestionError] = useState<string | null>(null);
  // Acting on a suggestion trades real risk, so it gates on the same narrow `hedge` × FI
  // capability that authoring the policy does. A viewer without it still SEES the standing
  // rows (they are risk information) but gets no act buttons.
  const canAct = app.auth.can("hedge", "fixed_income");

  // Load the engine config once (feeds the status strip + cap gauge).
  useEffect(() => {
    let cancelled = false;
    void app.transport
      .getHedgeConfig()
      .then((c) => !cancelled && setConfig(c))
      .catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, [app.transport]);

  // Subscribe to the live advisory-intent stream; refetch provenance on each tick so
  // the audit rows track the fired hedges (the engine appends provenance as it fires).
  useEffect(() => {
    let cancelled = false;
    const refetch = (): void => {
      void app.transport
        .listHedgeProvenance()
        .then((p) => !cancelled && setProvenance(p))
        .catch(() => undefined);
      // Standing suggestions are refetched on the SAME tick as the provenance: a
      // `suggest`-scoped breach raises a suggestion at exactly the moment an `auto`-scoped
      // one would have fired, so the intent push is the signal for both.
      void app.transport
        .listHedgeSuggestions()
        .then((s) => !cancelled && setSuggestions(s))
        .catch(() => undefined);
    };
    refetch();
    const dispose = app.transport.streamHedgeIntents((intent) => {
      if (cancelled) return;
      setIntents((cur) => [...cur, intent].slice(-40));
      refetch();
    });
    return () => {
      cancelled = true;
      dispose();
    };
  }, [app.transport]);

  /**
   * Act on a standing suggestion — with NO confirmation dialog on either path, which is
   * the whole point of the mode: the row already stood there long enough to BE the
   * deliberation, so re-asking would reintroduce the modal it exists to avoid.
   *
   * Optimistic: the row leaves at once. The reply then REPLACES the list (the server is
   * the authority on what still stands), and a failure restores the snapshot and shows the
   * error inline — so a rejected hedge never silently loses the suggestion.
   */
  const onActOnSuggestion = useCallback(
    (id: string, dismiss: boolean): void => {
      setSuggestionError(null);
      setBusySuggestions((cur) => new Set(cur).add(id));
      let snapshot: HedgeSuggestion[] = [];
      setSuggestions((cur) => {
        snapshot = cur;
        return cur.filter((s) => s.suggestionId !== id);
      });
      const clearBusy = (): void =>
        setBusySuggestions((cur) => {
          const next = new Set(cur);
          next.delete(id);
          return next;
        });
      void app.transport
        .executeHedgeSuggestion(id, dismiss)
        .then((reply) => {
          setSuggestions(reply.suggestions);
          // A fired hedge stamps provenance — pull it so the Live hedges table shows the
          // row the trader just created rather than waiting for the next intent tick.
          if (!dismiss) {
            void app.transport
              .listHedgeProvenance()
              .then(setProvenance)
              .catch(() => undefined);
          }
        })
        .catch((e: unknown) => {
          setSuggestions(snapshot);
          setSuggestionError(
            e instanceof Error
              ? e.message
              : `failed to ${dismiss ? "dismiss" : "execute"} the suggestion`,
          );
        })
        .finally(clearBusy);
    },
    [app.transport],
  );

  return (
    <HedgeMonitorView
      intents={intents}
      provenance={provenance}
      config={config}
      suggestions={suggestions}
      busySuggestions={busySuggestions}
      suggestionError={suggestionError}
      readOnly={!canAct}
      onActOnSuggestion={onActOnSuggestion}
    />
  );
}
