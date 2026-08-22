/**
 * DecisionAuditWorkspace — the trader-facing **audit log of the rule engines**.
 *
 * Three trader-configurable decision graphs gate every inbound fill: acceptance
 * (accept / reject / hold), risk routing (which book), and the hedge exit policy
 * (warehouse vs shed). Each of them used to leave a trace only when it ACTED. This
 * surface reads the server's decision journal, which records every evaluation —
 * including the ones that deliberately did nothing — so a trader can answer:
 *
 * * *why did acceptance turn that lift away* (or let it through)?
 * * *why did my hedge rule not fire on that position?*
 *
 * ## Two views of the same recorded rows
 *
 * The **table** sorts and filters the raw rows. The **inspector** shows, for the
 * selected row, the exact node path the server walked — rendered against the graph as
 * it stands now, with any node the graph no longer contains flagged stale rather than
 * relabelled — plus the utilisation-against-band history of that `(book, instrument)`
 * cell, so the portfolio state that produced the decision is visible beside it.
 *
 * ## Advice
 *
 * The advice rail is derived server-side from the same rows and cites them by `seq`.
 * Clicking the citation filters the table to exactly those rows, so a suggestion is
 * always one click from its evidence. Nothing here is modelled or extrapolated: an
 * empty journal produces an empty rail, and the surface says so.
 */
import { useCallback, useEffect, useMemo, useState } from "react";

import { HelpButton } from "../../components/HelpButton";
import type {
  AcceptanceGraph,
  DecisionEngine,
  DecisionOutcome,
  DecisionRecord,
  HedgeGraph,
  RuleAdvice,
} from "../../data/contract";
import { useApp, type WorkspaceId } from "../../app/AppContext";
import {
  ENGINE_LABEL,
  OUTCOME_LABEL,
  auditClock,
  filterRows,
  resolveWalkedPath,
  sortRows,
  utilizationSeries,
  windowCaveat,
  type AuditSort,
  type AuditSortKey,
} from "./decisionAudit";
import styles from "./DecisionAuditWorkspace.module.css";

const ENGINES: readonly DecisionEngine[] = ["acceptance", "risk_routing", "hedge"];
const OUTCOMES: readonly DecisionOutcome[] = ["fired", "no_action"];

/** The columns, in render order, with their header label and sort key. */
const COLUMNS: readonly { key: AuditSortKey; label: string; numeric?: boolean }[] = [
  { key: "seq", label: "#", numeric: true },
  { key: "engine", label: "Engine" },
  { key: "outcome", label: "Verdict" },
  { key: "book", label: "Book" },
  { key: "instrument", label: "Instrument" },
  { key: "counterparty", label: "Counterparty" },
  { key: "band", label: "Band" },
  { key: "utilization", label: "Util", numeric: true },
];

/** The rule editor an advice card deep-links to. */
const EDITOR_ROUTE: Record<string, WorkspaceId> = {
  hedging: "hedging",
  hedge_config: "hedging",
  hedge_vehicles: "hedging",
  acceptance: "acceptance",
  riskrouting: "riskrouting",
};

export function DecisionAuditWorkspace(): React.ReactElement {
  const { transport, setWorkspace } = useApp();

  const [engine, setEngine] = useState<DecisionEngine | undefined>();
  const [outcome, setOutcome] = useState<DecisionOutcome | undefined>();
  const [book, setBook] = useState("");
  const [query, setQuery] = useState("");
  const [evidenceSeqs, setEvidenceSeqs] = useState<readonly number[] | undefined>();
  const [sort, setSort] = useState<AuditSort>({ key: "seq", dir: "desc" });

  const [rows, setRows] = useState<DecisionRecord[]>([]);
  const [totalRecorded, setTotalRecorded] = useState(0);
  const [evicted, setEvicted] = useState(0);
  const [advice, setAdvice] = useState<RuleAdvice[]>([]);
  const [rowsConsidered, setRowsConsidered] = useState(0);
  const [selectedSeq, setSelectedSeq] = useState<number | null>(null);
  const [hedgeGraph, setHedgeGraph] = useState<HedgeGraph | null>(null);
  const [acceptanceGraph, setAcceptanceGraph] = useState<AcceptanceGraph | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);

  const refresh = useCallback(async (): Promise<void> => {
    setLoading(true);
    try {
      const [page, adv] = await Promise.all([
        transport.listDecisionJournal({
          engine,
          outcome,
          book: book.trim().length > 0 ? book.trim() : undefined,
        }),
        transport.listRuleAdvice(book.trim().length > 0 ? book.trim() : undefined),
      ]);
      setRows(page.records);
      setTotalRecorded(page.totalRecorded);
      setEvicted(page.evicted);
      setAdvice(adv.advice);
      setRowsConsidered(adv.rowsConsidered);
      setError(null);
    } catch (e) {
      // A failed audit query must READ as failed — never as "no decisions were taken".
      setError(e instanceof Error ? e.message : String(e));
      setRows([]);
      setAdvice([]);
    } finally {
      setLoading(false);
    }
  }, [transport, engine, outcome, book]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  // The CURRENT graphs, so a walked node id resolves to the rule a trader recognises.
  // A failure here degrades to unlabelled node ids (still honest) rather than blanking
  // the surface — the path itself is the recorded fact.
  useEffect(() => {
    let live = true;
    void (async () => {
      try {
        const [hg, ag] = await Promise.all([
          transport.getHedgePolicyGraph(),
          transport.getAcceptanceGraph(),
        ]);
        if (!live) return;
        setHedgeGraph(hg);
        setAcceptanceGraph(ag);
      } catch {
        if (!live) return;
        setHedgeGraph(null);
        setAcceptanceGraph(null);
      }
    })();
    return () => {
      live = false;
    };
  }, [transport]);

  const visible = useMemo(
    () => sortRows(filterRows(rows, { query, seqs: evidenceSeqs }), sort),
    [rows, query, evidenceSeqs, sort],
  );
  const selected = useMemo(
    () => visible.find((r) => r.seq === selectedSeq) ?? visible[0] ?? null,
    [visible, selectedSeq],
  );
  const walked = useMemo(() => {
    if (selected === null) return [];
    const graph = selected.engine === "acceptance" ? acceptanceGraph : hedgeGraph;
    return resolveWalkedPath(selected.policyPath, graph, selected.engine);
  }, [selected, hedgeGraph, acceptanceGraph]);
  const series = useMemo(
    () => (selected === null ? [] : utilizationSeries(rows, selected.book, selected.instrument)),
    [rows, selected],
  );
  const caveat = windowCaveat(totalRecorded, evicted);

  const toggleSort = (key: AuditSortKey): void =>
    setSort((s) => ({ key, dir: s.key === key && s.dir === "desc" ? "asc" : "desc" }));

  return (
    <div className={styles.shell} data-testid="decision-audit">
      <header className={styles.head}>
        <div>
          <h2 className={styles.title}>Decision audit</h2>
          <p className={styles.sub}>
            Every acceptance, routing and hedge evaluation the server recorded — including
            the ones that deliberately did nothing.
          </p>
        </div>
        <div className={styles.headActions}>
          <span className={styles.counter} data-testid="audit-counter">
            {visible.length.toLocaleString()} shown · {totalRecorded.toLocaleString()} recorded
          </span>
          <HelpButton helpId="concept.decision-audit" subject="the decision audit" />
        </div>
      </header>

      {caveat !== null && (
        <p className={styles.caveat} role="status" data-testid="audit-caveat">
          {caveat}
        </p>
      )}
      {error !== null && (
        <p className={styles.error} role="alert" data-testid="audit-error">
          The audit log could not be read: {error}
        </p>
      )}

      <div className={styles.filters} role="group" aria-label="Audit filters">
        <div className={styles.chipRow}>
          <span className={styles.chipLabel}>Engine</span>
          <button
            type="button"
            className={engine === undefined ? styles.chipOn : styles.chip}
            onClick={() => setEngine(undefined)}
            aria-pressed={engine === undefined}
          >
            All
          </button>
          {ENGINES.map((e) => (
            <button
              key={e}
              type="button"
              className={engine === e ? styles.chipOn : styles.chip}
              onClick={() => setEngine(e)}
              aria-pressed={engine === e}
            >
              {ENGINE_LABEL[e]}
            </button>
          ))}
        </div>
        <div className={styles.chipRow}>
          <span className={styles.chipLabel}>Verdict</span>
          <button
            type="button"
            className={outcome === undefined ? styles.chipOn : styles.chip}
            onClick={() => setOutcome(undefined)}
            aria-pressed={outcome === undefined}
          >
            All
          </button>
          {OUTCOMES.map((o) => (
            <button
              key={o}
              type="button"
              className={outcome === o ? styles.chipOn : styles.chip}
              onClick={() => setOutcome(o)}
              aria-pressed={outcome === o}
            >
              {OUTCOME_LABEL[o]}
            </button>
          ))}
        </div>
        <label className={styles.field}>
          <span>Book</span>
          <input value={book} onChange={(e) => setBook(e.target.value)} placeholder="all books" />
        </label>
        <label className={styles.field}>
          <span>Search</span>
          <input
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="reason, scope, counterparty…"
            data-testid="audit-search"
          />
        </label>
        {evidenceSeqs !== undefined && (
          <button
            type="button"
            className={styles.clearEvidence}
            onClick={() => setEvidenceSeqs(undefined)}
            data-testid="audit-clear-evidence"
          >
            Showing {evidenceSeqs.length} cited rows — clear
          </button>
        )}
      </div>

      <div className={styles.body}>
        <section className={styles.tableWrap} aria-label="Decision rows">
          <table className={styles.table}>
            <thead>
              <tr>
                {COLUMNS.map((c) => (
                  <th
                    key={c.key}
                    scope="col"
                    // `numeric` was declared on the column model and never applied, so a
                    // right-aligned value sat under a left-aligned label.
                    className={c.numeric === true ? styles.num : undefined}
                    aria-sort={
                      sort.key === c.key
                        ? sort.dir === "asc"
                          ? "ascending"
                          : "descending"
                        : "none"
                    }
                  >
                    <button type="button" onClick={() => toggleSort(c.key)}>
                      {c.label}
                      {sort.key === c.key && <span aria-hidden>{sort.dir === "asc" ? "▲" : "▼"}</span>}
                    </button>
                  </th>
                ))}
                <th scope="col">Reason</th>
              </tr>
            </thead>
            <tbody>
              {visible.map((r) => (
                <tr
                  key={r.seq}
                  className={selected?.seq === r.seq ? styles.rowOn : undefined}
                  onClick={() => setSelectedSeq(r.seq)}
                  data-testid={`audit-row-${r.seq}`}
                >
                  <td className={styles.num}>{r.seq}</td>
                  <td>{ENGINE_LABEL[r.engine]}</td>
                  <td>
                    <span
                      className={r.outcome === "fired" ? styles.verdictFired : styles.verdictIdle}
                    >
                      {r.outcomeLabel}
                    </span>
                    {r.advisory && <span className={styles.advisory}>advisory</span>}
                  </td>
                  <td>{r.book || "—"}</td>
                  <td>{r.instrument || "—"}</td>
                  <td>{r.counterparty ?? "—"}</td>
                  <td>
                    {r.band.length > 0 ? (
                      <span className={styles[`band_${r.band}`] ?? styles.bandNone}>{r.band}</span>
                    ) : (
                      "—"
                    )}
                  </td>
                  <td className={styles.num}>
                    {r.band.length > 0 ? `${(r.utilization * 100).toFixed(0)}%` : "—"}
                  </td>
                  <td className={styles.reason} title={r.reason}>
                    {r.reason}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
          {visible.length === 0 && (
            <p className={styles.empty} data-testid="audit-empty">
              {loading
                ? "Reading the decision journal…"
                : totalRecorded === 0
                  ? "No decisions have been recorded yet. Rows appear as inbound flow is evaluated by the acceptance, routing and hedge policies."
                  : "No recorded decision matches these filters."}
            </p>
          )}
        </section>

        <aside className={styles.inspector} aria-label="Decision detail">
          {selected === null ? (
            <p className={styles.empty}>Select a decision to see why it went the way it did.</p>
          ) : (
            <>
              <h3 className={styles.inspTitle}>
                #{selected.seq} · {ENGINE_LABEL[selected.engine]} ·{" "}
                {auditClock(selected.decidedAtNanos)}
              </h3>
              <p className={styles.inspReason}>{selected.reason}</p>
              <dl className={styles.facts}>
                <div>
                  <dt>Policy in force</dt>
                  <dd>{selected.scope || "—"}</dd>
                </div>
                {selected.band.length > 0 && (
                  <div>
                    <dt>Risk at decision</dt>
                    <dd>
                      {selected.netRisk.toLocaleString(undefined, { maximumFractionDigits: 0 })} of{" "}
                      {selected.threshold.toLocaleString(undefined, { maximumFractionDigits: 0 })} (
                      {(selected.utilization * 100).toFixed(0)}%, {selected.band})
                    </dd>
                  </div>
                )}
                {selected.hedgeId !== undefined && (
                  <div>
                    <dt>Hedge record</dt>
                    <dd>{selected.hedgeId}</dd>
                  </div>
                )}
                {selected.traceId !== undefined && (
                  <div>
                    <dt>Lift trace</dt>
                    <dd>{selected.traceId}</dd>
                  </div>
                )}
              </dl>

              <h4 className={styles.inspHead}>Path walked</h4>
              {walked.length === 0 ? (
                <p className={styles.noPath} data-testid="audit-no-path">
                  No graph was walked for this decision — see the reason above.
                </p>
              ) : (
                <ol className={styles.path} data-testid="audit-path">
                  {walked.map((step) => (
                    <li
                      key={step.id}
                      className={
                        step.stale ? styles.stepStale : step.leaf ? styles.stepLeaf : styles.step
                      }
                    >
                      <span className={styles.stepId}>node {step.id}</span>
                      <span className={styles.stepLabel}>
                        {step.label ?? "no longer in the current graph"}
                      </span>
                    </li>
                  ))}
                </ol>
              )}

              {series.length > 1 && (
                <>
                  <h4 className={styles.inspHead}>
                    Utilisation against the band — {selected.book} · {selected.instrument}
                  </h4>
                  <UtilizationChart
                    points={series}
                    highlightSeq={selected.seq}
                    testId="audit-series"
                  />
                </>
              )}
            </>
          )}
        </aside>
      </div>

      <section className={styles.advice} aria-label="Derived rule suggestions">
        <h3 className={styles.inspHead}>
          Suggested rules{" "}
          <span className={styles.denominator}>
            derived from {rowsConsidered.toLocaleString()} recorded decisions
          </span>
        </h3>
        {advice.length === 0 ? (
          <p className={styles.empty} data-testid="advice-empty">
            {rowsConsidered === 0
              ? "Nothing to derive from yet — suggestions appear once decisions have been recorded."
              : "No recurring pattern in the recorded decisions warrants a rule change."}
          </p>
        ) : (
          <ul className={styles.adviceList}>
            {advice.map((a) => (
              <li key={a.adviceId} className={styles.adviceCard} data-testid={`advice-${a.kind}`}>
                <header>
                  <h4>{a.title}</h4>
                  <span className={styles.occurrences}>{a.occurrences} occurrences</span>
                </header>
                <p className={styles.rationale}>{a.rationale}</p>
                <p className={styles.action}>{a.recommendedAction}</p>
                <div className={styles.adviceFoot}>
                  <button
                    type="button"
                    className={styles.evidenceBtn}
                    onClick={() => {
                      setEvidenceSeqs(a.evidenceSeqs);
                      setSelectedSeq(a.evidenceSeqs[0] ?? null);
                    }}
                    data-testid={`advice-evidence-${a.kind}`}
                  >
                    Show the {a.evidenceSeqs.length} cited rows
                  </button>
                  {/*
                   * Navigate through the app's OWN workspace switch. This was an
                   * `<a href="#/hedging">`, but nothing in the app reads the URL
                   * fragment — the only `window.location.hash` reference merely
                   * PRESERVES it while rewriting query params — so the link set a
                   * fragment nobody listens to and the button appeared dead. Every
                   * target in EDITOR_ROUTE is a real WorkspaceId, so the route map
                   * was right; only the mechanism was wrong.
                   */}
                  <button
                    type="button"
                    className={styles.editorLink}
                    onClick={() => setWorkspace(EDITOR_ROUTE[a.editor] ?? "hedging")}
                    data-testid={`advice-editor-${a.kind}`}
                  >
                    Open the rule editor
                  </button>
                </div>
              </li>
            ))}
          </ul>
        )}
      </section>
    </div>
  );
}

/**
 * The utilisation-against-band series as an inline SVG sparkline. Amber (0.8) and red
 * (0.9) guides are drawn as reference lines so a trader reads each decision against the
 * band it was taken in. Every point is a recorded decision — no interpolation.
 */
function UtilizationChart({
  points,
  highlightSeq,
  testId,
}: {
  points: readonly { seq: number; utilization: number; acted: boolean }[];
  highlightSeq: number;
  testId: string;
}): React.ReactElement {
  const W = 460;
  const H = 96;
  const max = Math.max(1.1, ...points.map((p) => p.utilization));
  const x = (i: number): number =>
    points.length === 1 ? W / 2 : (i / (points.length - 1)) * (W - 16) + 8;
  const y = (u: number): number => H - 8 - (u / max) * (H - 20);
  const line = points.map((p, i) => `${i === 0 ? "M" : "L"}${x(i)},${y(p.utilization)}`).join(" ");
  return (
    <svg
      className={styles.chart}
      viewBox={`0 0 ${W} ${H}`}
      role="img"
      aria-label={`Utilisation across ${points.length} recorded decisions`}
      data-testid={testId}
    >
      <line className={styles.guideAmber} x1={0} x2={W} y1={y(0.8)} y2={y(0.8)} />
      <line className={styles.guideRed} x1={0} x2={W} y1={y(0.9)} y2={y(0.9)} />
      <path className={styles.spark} d={line} />
      {points.map((p, i) => (
        <circle
          key={p.seq}
          className={
            p.seq === highlightSeq ? styles.dotOn : p.acted ? styles.dotActed : styles.dotIdle
          }
          cx={x(i)}
          cy={y(p.utilization)}
          r={p.seq === highlightSeq ? 4 : 2.5}
        />
      ))}
    </svg>
  );
}
