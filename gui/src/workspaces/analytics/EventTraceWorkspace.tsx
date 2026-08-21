/**
 * EventTraceWorkspace — the per-lift EVENT TRACE view: a timeline that stitches one
 * lift's whole life under a single `traceId` — price → quote → order → last-look →
 * acceptance → risk-route → book → hedge-decide → hedge-fire — showing the
 * inter-stage latency of every hop and each stage's details. The per-trace
 * complement to the aggregate Latency/Ops histograms (one lift, end to end, rather
 * than a percentile digest across many).
 *
 * Two panes:
 *   • Trace list — recent traces (newest first) from `listTraces()`, filterable by
 *     symbol / counterparty, each row a sortable summary (id, symbol, counterparty,
 *     outcome, total latency, event count, last stage). Selecting a row loads its
 *     timeline.
 *   • Timeline — the selected trace's ordered stages from `getTrace()`, each with its
 *     absolute time, the Δ latency from the previous stage (adaptive ns/µs/ms) drawn
 *     as a proportional bar, and its stage details. The single biggest-Δ hop — the
 *     slowest step — is emphasised.
 *
 * Live: revalidated on the notification bus (trailing-debounced) + a low idle poll,
 * exactly like the Latency/Ops table. Read-only and gated on `view_analytics`.
 *
 * Blotter deep-link: consumes `app.traceFocus` — a `{kind:"trace"}` focus loads that
 * trace directly; a `{kind:"position"}` focus (from a Deals-blotter "View trace"
 * action) has NO direct server position→trace lookup, so it SCANS the listed page of
 * traces for the event carrying the matching `positionId`. If the trace has aged off
 * the listed page it is not found (a graceful "no trace" state, list still shown).
 */
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { useApp } from "../../app/AppContext";
import { TableSearch } from "../../components/TableSearch";
import type { TraceFocus } from "../../app/AppContext";
import type { TraceEvent, TraceOutcome, TraceSummary } from "../../data/contract";
import { traceStageLabel } from "../../data/contract";
import { fmtClock } from "../../lib/format";
import { fmtTraceLatency, interStageLatencies } from "../../lib/traceLatency";
import styles from "./EventTraceWorkspace.module.css";

// --- live-refresh cadence (mirrors LatencyOpsWorkspace) ----------------------

/** Trailing-debounce for the notification-driven list revalidate. */
const REVALIDATE_DEBOUNCE_MS = 750;
/** Idle poll so the recent-traces list stays fresh with no notification traffic. */
const IDLE_REFRESH_MS = 5_000;
/** Debounce for the symbol / counterparty filter inputs before a refetch. */
const FILTER_DEBOUNCE_MS = 300;

// --- outcome badge -----------------------------------------------------------

/** Human label + tone class for a trace outcome (real text, never colour-only). */
const OUTCOME_LABEL: Record<TraceOutcome, string> = {
  booked: "Booked",
  hedged: "Hedged",
  rejected: "Rejected",
  in_flight: "In flight",
};

function outcomeToneClass(outcome: TraceOutcome, s: typeof styles): string {
  switch (outcome) {
    case "hedged":
      return s.outHedged ?? "";
    case "booked":
      return s.outBooked ?? "";
    case "rejected":
      return s.outRejected ?? "";
    case "in_flight":
    default:
      return s.outInFlight ?? "";
  }
}

function OutcomeBadge({ outcome }: { outcome: TraceOutcome }): React.ReactElement {
  return (
    <span className={`${styles.outcome} ${outcomeToneClass(outcome, styles)}`}>
      {OUTCOME_LABEL[outcome]}
    </span>
  );
}

// --- list sort model ---------------------------------------------------------

type SortDir = "asc" | "desc";

interface ColumnDef {
  key: string;
  label: string;
  title: string;
  numeric: boolean;
  sortValue: (t: TraceSummary) => number | string;
}

const COLUMNS: readonly ColumnDef[] = [
  { key: "traceId", label: "Trace", title: "Trace id", numeric: false, sortValue: (t) => Number(t.traceId) },
  { key: "symbol", label: "Symbol", title: "Instrument symbol", numeric: false, sortValue: (t) => t.symbol },
  { key: "counterparty", label: "Counterparty", title: "Originating counterparty", numeric: false, sortValue: (t) => t.counterparty ?? "" },
  { key: "outcome", label: "Outcome", title: "Terminal outcome", numeric: false, sortValue: (t) => t.outcome },
  { key: "totalLatencyNs", label: "Total latency", title: "End-to-end latency (last − first stage)", numeric: true, sortValue: (t) => t.totalLatencyNs },
  { key: "eventCount", label: "Stages", title: "Stage events captured", numeric: true, sortValue: (t) => t.eventCount },
  { key: "lastStage", label: "Last stage", title: "How far the lift progressed", numeric: false, sortValue: (t) => t.lastStage },
];

function compareSort(a: number | string, b: number | string, dir: SortDir): number {
  let cmp: number;
  if (typeof a === "string" || typeof b === "string") cmp = String(a).localeCompare(String(b));
  else cmp = a - b;
  return dir === "asc" ? cmp : -cmp;
}

export function EventTraceWorkspace(): React.ReactElement {
  const app = useApp();
  const { auth } = app;
  const signedIn = auth.user !== undefined && auth.user !== null;

  const [summaries, setSummaries] = useState<TraceSummary[] | null>(null);
  const [listError, setListError] = useState<string | null>(null);
  const [listLoading, setListLoading] = useState(false);

  const [symbolInput, setSymbolInput] = useState("");
  const [cpInput, setCpInput] = useState("");
  const [appliedSymbol, setAppliedSymbol] = useState("");
  const [appliedCp, setAppliedCp] = useState("");

  const [selectedId, setSelectedId] = useState<bigint | null>(null);
  const [events, setEvents] = useState<TraceEvent[] | null>(null);
  const [timelineError, setTimelineError] = useState<string | null>(null);
  const [timelineLoading, setTimelineLoading] = useState(false);
  // Set when a blotter position-focus resolves to no trace on the listed page.
  const [focusNotFound, setFocusNotFound] = useState<string | null>(null);

  const [sortKey, setSortKey] = useState<string | null>(null);
  const [sortDir, setSortDir] = useState<SortDir>("desc");
  // Distinct from the symbol / counterparty boxes above, which are SERVER filters that
  // re-query. This one narrows the traces already on screen without a round trip, and
  // reaches fields the server filters do not expose — the outcome, the last stage, and
  // the trace id itself, which is what you have when someone sends you one.
  const [query, setQuery] = useState("");
  // Outcome is a small closed vocabulary and the first thing anyone triages on.
  const [outcomeFilter, setOutcomeFilter] = useState("");

  // Debounce the filter inputs into the applied (fetched) filter values.
  useEffect(() => {
    const id = setTimeout(() => {
      setAppliedSymbol(symbolInput.trim());
      setAppliedCp(cpInput.trim());
    }, FILTER_DEBOUNCE_MS);
    return () => clearTimeout(id);
  }, [symbolInput, cpInput]);

  const loadList = useCallback((): void => {
    if (!signedIn) {
      setSummaries(null);
      setListError(null);
      return;
    }
    setListLoading(true);
    const filter = {
      ...(appliedSymbol.length > 0 ? { symbol: appliedSymbol } : {}),
      ...(appliedCp.length > 0 ? { counterparty: appliedCp } : {}),
    };
    void app.transport
      .listTraces(filter)
      .then((rows) => {
        setSummaries(rows);
        setListError(null);
      })
      .catch((e: unknown) => {
        setSummaries(null);
        setListError(e instanceof Error ? e.message : "failed to load traces");
      })
      .finally(() => setListLoading(false));
  }, [app.transport, signedIn, appliedSymbol, appliedCp]);

  useEffect(() => {
    loadList();
  }, [loadList]);

  // Live refresh: notification-driven revalidate (trailing-debounced) + idle poll,
  // the same seam the Latency/Ops table uses. Best-effort on the push stream.
  useEffect(() => {
    if (!signedIn) return;
    let debounce: ReturnType<typeof setTimeout> | undefined;
    const revalidate = (): void => {
      if (debounce !== undefined) clearTimeout(debounce);
      debounce = setTimeout(() => {
        debounce = undefined;
        loadList();
      }, REVALIDATE_DEBOUNCE_MS);
    };
    const interval = setInterval(loadList, IDLE_REFRESH_MS);
    const stream = app.transport.streamNotifications;
    const dispose =
      typeof stream === "function" ? stream.call(app.transport, undefined, revalidate) : undefined;
    return () => {
      if (debounce !== undefined) clearTimeout(debounce);
      clearInterval(interval);
      dispose?.();
    };
  }, [app.transport, signedIn, loadList]);

  const loadTimeline = useCallback(
    (traceId: bigint): void => {
      setTimelineLoading(true);
      setTimelineError(null);
      void app.transport
        .getTrace(traceId)
        .then((evs) => {
          setEvents(evs);
          setTimelineError(null);
        })
        .catch((e: unknown) => {
          setEvents(null);
          setTimelineError(e instanceof Error ? e.message : "failed to load trace");
        })
        .finally(() => setTimelineLoading(false));
    },
    [app.transport],
  );

  const onSelect = useCallback(
    (traceId: bigint): void => {
      setSelectedId(traceId);
      setFocusNotFound(null);
      loadTimeline(traceId);
    },
    [loadTimeline],
  );

  // Consume a blotter deep-link focus (STEP 5). A `trace` focus loads directly; a
  // `position` focus has no server lookup, so we SCAN the listed page for the event
  // carrying the matching positionId (bounded to the fetched page — see the in-UI
  // hint). The focus is cleared once consumed so it fires exactly once.
  const focusRef = useRef<TraceFocus | null>(null);
  useEffect(() => {
    const focus = app.traceFocus;
    if (focus === null) return;
    if (focusRef.current === focus) return; // guard against a re-run on the same focus
    focusRef.current = focus;
    let cancelled = false;
    if (focus.kind === "trace") {
      setSelectedId(focus.traceId);
      setFocusNotFound(null);
      loadTimeline(focus.traceId);
      app.clearTraceFocus();
      return;
    }
    // kind === "position": resolve via a listed-page scan.
    setTimelineLoading(true);
    setFocusNotFound(null);
    void (async () => {
      try {
        const rows = await app.transport.listTraces();
        for (const row of rows) {
          if (cancelled) return;
          const evs = await app.transport.getTrace(row.traceId);
          if (evs.some((e) => e.positionId === focus.positionId)) {
            if (cancelled) return;
            setSelectedId(row.traceId);
            setEvents(evs);
            setTimelineError(null);
            setTimelineLoading(false);
            app.clearTraceFocus();
            return;
          }
        }
        if (cancelled) return;
        setEvents(null);
        setFocusNotFound(
          focus.label !== undefined
            ? `No trace found for this deal (${focus.label}).`
            : "No trace found for this deal.",
        );
        setTimelineLoading(false);
        app.clearTraceFocus();
      } catch (e: unknown) {
        if (cancelled) return;
        setTimelineError(e instanceof Error ? e.message : "failed to resolve trace");
        setTimelineLoading(false);
        app.clearTraceFocus();
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [app.traceFocus, app.transport, app.clearTraceFocus, loadTimeline]);

  const onSort = useCallback((key: string): void => {
    setSortKey((prev) => {
      if (prev === key) {
        setSortDir((d) => (d === "asc" ? "desc" : "asc"));
        return prev;
      }
      setSortDir("desc");
      return key;
    });
  }, []);

  const rows = summaries ?? [];

  /** Every distinct outcome present, so the filter offers only reachable values. */
  const outcomeOptions = useMemo(
    () => [...new Set(rows.map((t) => t.outcome))].sort((a, b) => a.localeCompare(b)),
    [rows],
  );

  const matchedRows = useMemo(() => {
    const needle = query.trim().toLowerCase();
    return rows.filter((t) => {
      if (outcomeFilter !== "" && t.outcome !== outcomeFilter) return false;
      if (needle === "") return true;
      return [
        String(t.traceId),
        t.symbol,
        t.counterparty ?? "",
        t.outcome,
        t.lastStage,
      ]
        .join(" ")
        .toLowerCase()
        .includes(needle);
    });
  }, [rows, query, outcomeFilter]);

  const visibleRows = useMemo(() => {
    if (sortKey === null) return matchedRows; // preserve server order (newest first)
    const col = COLUMNS.find((c) => c.key === sortKey);
    if (!col) return matchedRows;
    return [...matchedRows].sort((a, b) =>
      compareSort(col.sortValue(a), col.sortValue(b), sortDir),
    );
  }, [matchedRows, sortKey, sortDir]);

  const ariaSort = (key: string): "ascending" | "descending" | "none" =>
    sortKey === key ? (sortDir === "asc" ? "ascending" : "descending") : "none";

  const selectedSummary = useMemo(
    () => (selectedId === null ? undefined : rows.find((r) => r.traceId === selectedId)),
    [rows, selectedId],
  );

  return (
    <section className={styles.wrap} aria-labelledby="eventtrace-title">
      <header className={styles.head}>
        <div className={styles.headMain}>
          <h1 id="eventtrace-title" className={styles.title}>
            Event Trace
          </h1>
          <p className={styles.subtitle}>
            One lift, end to end — price → order → acceptance → risk → hedge under a single
            trace id, with the latency of every hop and each stage&apos;s details.
          </p>
        </div>
      </header>

      <p className={styles.explainer}>
        A <strong>trace</strong> links every stage of one lift&apos;s life under a shared
        id: <strong>Price computed → Quote published → Order received → Last look →
        Acceptance decided → Risk routed → Deal booked → Hedge decided → Hedge fired</strong>.
        Each stage is timestamped, so the <strong>Δ latency</strong> between two stages is
        just the gap between their timestamps — the timeline draws each hop to scale and
        marks the <strong>slowest</strong> one. Select a trace on the left to open it.
      </p>

      {!signedIn ? (
        <p className={styles.empty}>Sign in to view event traces.</p>
      ) : (
        <div className={styles.panes}>
          <TraceList
            rows={visibleRows}
            totalRows={rows.length}
            query={query}
            onQueryChange={setQuery}
            outcomeFilter={outcomeFilter}
            onOutcomeFilter={setOutcomeFilter}
            outcomeOptions={outcomeOptions}
            loading={listLoading}
            error={listError}
            symbolInput={symbolInput}
            cpInput={cpInput}
            onSymbolInput={setSymbolInput}
            onCpInput={setCpInput}
            selectedId={selectedId}
            onSelect={onSelect}
            onSort={onSort}
            sortKey={sortKey}
            sortDir={sortDir}
            ariaSort={ariaSort}
          />
          <TraceTimeline
            events={events}
            loading={timelineLoading}
            error={timelineError}
            focusNotFound={focusNotFound}
            summary={selectedSummary}
          />
        </div>
      )}
    </section>
  );
}

// --- trace list --------------------------------------------------------------

interface TraceListProps {
  rows: readonly TraceSummary[];
  /** Total traces loaded, before the client-side search — the "M" in "N of M". */
  totalRows: number;
  query: string;
  onQueryChange: (v: string) => void;
  outcomeFilter: string;
  onOutcomeFilter: (v: string) => void;
  /** Only the outcomes actually present, so the select never offers a dead option. */
  outcomeOptions: readonly string[];
  loading: boolean;
  error: string | null;
  symbolInput: string;
  cpInput: string;
  onSymbolInput: (v: string) => void;
  onCpInput: (v: string) => void;
  selectedId: bigint | null;
  onSelect: (traceId: bigint) => void;
  onSort: (key: string) => void;
  sortKey: string | null;
  sortDir: SortDir;
  ariaSort: (key: string) => "ascending" | "descending" | "none";
}

function TraceList({
  rows,
  totalRows,
  query,
  onQueryChange,
  outcomeFilter,
  onOutcomeFilter,
  outcomeOptions,
  loading,
  error,
  symbolInput,
  cpInput,
  onSymbolInput,
  onCpInput,
  selectedId,
  onSelect,
  onSort,
  sortKey,
  sortDir,
  ariaSort,
}: TraceListProps): React.ReactElement {
  return (
    <div className={styles.listPane}>
      <div className={styles.filters}>
        <label className={styles.filter}>
          <span className={styles.filterLabel}>Symbol</span>
          <input
            type="text"
            className={styles.filterInput}
            value={symbolInput}
            onChange={(e) => onSymbolInput(e.target.value)}
            placeholder="e.g. EURUSD"
            spellCheck={false}
            autoComplete="off"
          />
        </label>
        <label className={styles.filter}>
          <span className={styles.filterLabel}>Counterparty</span>
          <input
            type="text"
            className={styles.filterInput}
            value={cpInput}
            onChange={(e) => onCpInput(e.target.value)}
            placeholder="e.g. cp-alpha"
            spellCheck={false}
            autoComplete="off"
          />
        </label>
      </div>

      {error !== null && (
        <p className={styles.error} role="alert">
          {error}
        </p>
      )}

      {loading && rows.length === 0 ? (
        <p className={styles.empty}>Loading traces…</p>
      ) : rows.length === 0 ? (
        <p className={styles.empty}>No traces match.</p>
      ) : (
        <>
        <div className={styles.tableTools}>
          <TableSearch
            query={query}
            onQueryChange={onQueryChange}
            shown={rows.length}
            total={totalRows}
            label="Search traces"
            placeholder="Filter by trace id, symbol, outcome or stage…"
          />
          <label className={styles.outcomeFilter}>
            <span className={styles.outcomeFilterLabel}>Outcome</span>
            <select
              className={styles.outcomeFilterSelect}
              value={outcomeFilter}
              aria-label="Filter by trace outcome"
              data-testid="event-trace-outcome-filter"
              onChange={(e) => onOutcomeFilter(e.target.value)}
            >
              <option value="">Any outcome</option>
              {outcomeOptions.map((o) => (
                <option key={o} value={o}>
                  {o}
                </option>
              ))}
            </select>
          </label>
        </div>
        <div className={styles.tableScroll}>
          <table className={styles.table} data-testid="event-trace-list">
            <caption className={styles.caption}>Recent traces — newest first</caption>
            <thead>
              <tr>
                {COLUMNS.map((c) => (
                  <th
                    key={c.key}
                    scope="col"
                    className={c.numeric ? styles.thNum : styles.thText}
                    aria-sort={ariaSort(c.key)}
                    title={c.title}
                  >
                    <button
                      type="button"
                      className={c.numeric ? styles.sortBtnNum : styles.sortBtn}
                      onClick={() => onSort(c.key)}
                    >
                      {c.label}
                      <SortGlyph active={sortKey === c.key} dir={sortDir} />
                    </button>
                  </th>
                ))}
              </tr>
            </thead>
            <tbody>
              {rows.map((t) => {
                const isSel = selectedId === t.traceId;
                return (
                  <tr
                    key={t.traceId.toString()}
                    className={isSel ? `${styles.row} ${styles.rowSelected}` : styles.row}
                    data-testid="trace-row"
                  >
                    <th scope="row" className={styles.rowHead}>
                      <button
                        type="button"
                        className={styles.selectBtn}
                        aria-pressed={isSel}
                        onClick={() => onSelect(t.traceId)}
                      >
                        #{t.traceId.toString()}
                      </button>
                    </th>
                    <td className={styles.cell}>{t.symbol}</td>
                    <td className={styles.cell}>{t.counterparty ?? "—"}</td>
                    <td className={styles.cell}>
                      <OutcomeBadge outcome={t.outcome} />
                    </td>
                    <td className={styles.num}>{fmtTraceLatency(t.totalLatencyNs)}</td>
                    <td className={styles.num}>{t.eventCount}</td>
                    <td className={styles.cell}>{traceStageLabel(t.lastStage)}</td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
        </>
      )}
    </div>
  );
}

// --- trace timeline ----------------------------------------------------------

interface TraceTimelineProps {
  events: TraceEvent[] | null;
  loading: boolean;
  error: string | null;
  focusNotFound: string | null;
  summary: TraceSummary | undefined;
}

function TraceTimeline({
  events,
  loading,
  error,
  focusNotFound,
  summary,
}: TraceTimelineProps): React.ReactElement {
  const latencies = useMemo(() => (events ? interStageLatencies(events) : []), [events]);
  const maxDelta = useMemo(
    () => Math.max(1, ...latencies.map((l) => l.deltaNs ?? 0)),
    [latencies],
  );

  return (
    <div className={styles.timelinePane} aria-labelledby="eventtrace-timeline-title">
      <h2 id="eventtrace-timeline-title" className={styles.paneTitle}>
        Timeline
        {summary && (
          <span className={styles.paneTitleSub}>
            {" "}
            #{summary.traceId.toString()} · {summary.symbol}
          </span>
        )}
      </h2>

      {error !== null && (
        <p className={styles.error} role="alert">
          {error}
        </p>
      )}

      {focusNotFound !== null && (
        <p className={styles.notice} role="status">
          {focusNotFound} The trace may have aged off the recent-traces page (there is no
          direct deal→trace lookup — the link scans the listed page). Pick a trace from the
          list to inspect it.
        </p>
      )}

      {loading && events === null ? (
        <p className={styles.empty}>Loading timeline…</p>
      ) : events === null ? (
        focusNotFound === null && <p className={styles.empty}>Select a trace to see its timeline.</p>
      ) : events.length === 0 ? (
        <p className={styles.empty}>This trace has no events (it may have been evicted).</p>
      ) : (
        <ol className={styles.timeline} data-testid="trace-timeline">
          {events.map((ev, i) => (
            <StageRow
              key={ev.seq}
              event={ev}
              latency={latencies[i]!}
              maxDelta={maxDelta}
              isFirst={i === 0}
            />
          ))}
        </ol>
      )}
    </div>
  );
}

const PRICE_FMT = new Intl.NumberFormat("en-US", { maximumFractionDigits: 6 });
const NOTIONAL_FMT = new Intl.NumberFormat("en-US", { notation: "compact", maximumFractionDigits: 2 });

/** A present optional value, or "—" (a dim placeholder) when absent. */
function optText(v: string | undefined): { value: string; absent: boolean } {
  return v !== undefined && v.length > 0 ? { value: v, absent: false } : { value: "—", absent: true };
}

function StageRow({
  event,
  latency,
  maxDelta,
  isFirst,
}: {
  event: TraceEvent;
  latency: { deltaNs: number | undefined; isMax: boolean };
  maxDelta: number;
  isFirst: boolean;
}): React.ReactElement {
  const pct = latency.deltaNs !== undefined ? Math.max(2, (latency.deltaNs / maxDelta) * 100) : 0;
  const detail: readonly { label: string; value: string; absent: boolean }[] = [
    { label: "Side", ...optText(event.side) },
    {
      label: "Price",
      value: event.price !== undefined ? PRICE_FMT.format(event.price) : "—",
      absent: event.price === undefined,
    },
    {
      label: "Notional",
      value: event.notional !== undefined ? NOTIONAL_FMT.format(event.notional) : "—",
      absent: event.notional === undefined,
    },
    { label: "Counterparty", ...optText(event.counterparty) },
    { label: "Quote id", ...optText(event.quoteId) },
    { label: "Deal id", ...optText(event.dealId) },
    { label: "Book", ...optText(event.bookId) },
    { label: "Decision", ...optText(event.decision) },
    { label: "Hedge id", ...optText(event.hedgeId) },
    {
      label: "Position",
      value: event.positionId !== undefined ? `#${event.positionId.toString()}` : "—",
      absent: event.positionId === undefined,
    },
    { label: "Detail", ...optText(event.detail) },
  ];
  return (
    <li className={latency.isMax ? `${styles.stage} ${styles.stageMax}` : styles.stage}>
      <div className={styles.stageRail} aria-hidden>
        <span className={styles.stageDot} />
      </div>
      <div className={styles.stageBody}>
        <div className={styles.stageHead}>
          <span className={styles.stageName}>{traceStageLabel(event.stage)}</span>
          <span className={styles.stageClock}>{fmtClock(event.timestampNs)}</span>
          <span className={styles.stageDelta}>
            {isFirst ? (
              <span className={styles.deltaStart}>start</span>
            ) : (
              <>
                <span className={styles.deltaLabel}>+</span>
                {fmtTraceLatency(latency.deltaNs)}
              </>
            )}
          </span>
          {latency.isMax && <span className={styles.maxPill}>slowest hop</span>}
        </div>
        <div className={styles.stageBarTrack} aria-hidden>
          <span className={styles.stageBarFill} style={{ width: `${pct}%` }} />
        </div>
        <dl className={styles.details}>
          {detail.map((d) => (
            <div className={styles.detailField} key={d.label}>
              <dt className={styles.detailLabel}>{d.label}</dt>
              <dd className={d.absent ? `${styles.detailValue} ${styles.detailAbsent}` : styles.detailValue}>
                {d.value}
              </dd>
            </div>
          ))}
        </dl>
      </div>
    </li>
  );
}

function SortGlyph({ active, dir }: { active: boolean; dir: SortDir }): React.ReactElement {
  return (
    <span className={styles.sortGlyph} aria-hidden>
      {active ? (dir === "asc" ? "▲" : "▼") : "↕"}
    </span>
  );
}
