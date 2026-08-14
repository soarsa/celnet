/**
 * Pure presentation logic for the Decision Audit surface — sorting, filtering, the
 * band/utilisation series, and the walked-path resolution against the CURRENT graphs.
 *
 * Kept free of React so the sort/filter contract and the honesty rules below are unit
 * tested directly rather than through the DOM.
 *
 * ## The honesty rules encoded here
 *
 * 1. A walked node id is resolved against the graph as it is **now**. A graph can be
 *    edited after a decision was taken, so {@link resolveWalkedPath} marks any node the
 *    current graph no longer contains as `stale` rather than inventing a label for it.
 * 2. The utilisation series is built from the recorded rows only. There is no
 *    interpolation and no synthetic "current" point — if the journal window holds three
 *    rows, the chart shows three points.
 */

import type {
  AcceptanceGraph,
  AcceptanceNode,
  DecisionEngine,
  DecisionOutcome,
  DecisionRecord,
  HedgeGraph,
  HedgeNode,
} from "../../data/contract";
import { acceptanceActionLabel } from "../../lib/acceptanceAction";
import { opGlyph } from "../../lib/routeFields";
import { valueLabel } from "../riskrouting/nodeLabel";

/** A field enum member (`net_dv01`, `quote_age_ms`) as a readable label. */
function fieldLabel(field: string): string {
  return field.replace(/_/g, " ");
}

/** The columns the audit table can be ordered by. */
export type AuditSortKey =
  | "seq"
  | "engine"
  | "outcome"
  | "book"
  | "instrument"
  | "counterparty"
  | "band"
  | "utilization";

/** A sort request: which column, which direction. */
export interface AuditSort {
  key: AuditSortKey;
  dir: "asc" | "desc";
}

/** The client-side narrowing applied ON TOP of the server-side query filters. */
export interface AuditTextFilter {
  /** Free text matched (case-insensitively) against the reason, labels and scope. */
  query: string;
  /** Restrict to these `seq` values — how an advice card's evidence is drilled into. */
  seqs?: readonly number[] | undefined;
}

/** Human labels for the engines (used by the chips, the table and the detail panel). */
export const ENGINE_LABEL: Record<DecisionEngine, string> = {
  acceptance: "Acceptance",
  risk_routing: "Risk routing",
  hedge: "Hedge policy",
};

/** Human labels for the outcome filter chips. */
export const OUTCOME_LABEL: Record<DecisionOutcome, string> = {
  fired: "Acted",
  no_action: "Did nothing",
};

/** Epoch nanos → a 24h local clock string. */
export function auditClock(nanos: number): string {
  return new Date(nanos / 1e6).toLocaleTimeString("en-GB", { hour12: false });
}

/** The searchable text of one row — everything the free-text box matches against. */
function haystack(r: DecisionRecord): string {
  return [
    r.reason,
    r.outcomeLabel,
    r.scope,
    r.book,
    r.instrument,
    r.desk,
    r.counterparty ?? "",
    r.symbol ?? "",
    r.hedgeId ?? "",
    r.requestId ?? "",
  ]
    .join(" ")
    .toLowerCase();
}

/** Apply the client-side text / evidence narrowing. */
export function filterRows(
  rows: readonly DecisionRecord[],
  filter: AuditTextFilter,
): DecisionRecord[] {
  const q = filter.query.trim().toLowerCase();
  const seqs = filter.seqs === undefined ? undefined : new Set(filter.seqs);
  return rows.filter((r) => {
    if (seqs !== undefined && !seqs.has(r.seq)) return false;
    if (q.length === 0) return true;
    return haystack(r).includes(q);
  });
}

/** The value one sort key reads off a row (strings compare lexically, numbers numerically). */
function sortValue(r: DecisionRecord, key: AuditSortKey): string | number {
  switch (key) {
    case "seq":
      return r.seq;
    case "engine":
      return ENGINE_LABEL[r.engine];
    case "outcome":
      return r.outcomeLabel;
    case "book":
      return r.book;
    case "instrument":
      return r.instrument;
    case "counterparty":
      return r.counterparty ?? "";
    case "band":
      return r.band;
    case "utilization":
      return r.utilization;
  }
}

/**
 * Order the rows. The sort is TOTAL — ties break on the (unique, monotonic) `seq`, so a
 * re-render never reshuffles equal rows and a trader's eye keeps its place.
 */
export function sortRows(rows: readonly DecisionRecord[], sort: AuditSort): DecisionRecord[] {
  const sign = sort.dir === "asc" ? 1 : -1;
  return [...rows].sort((a, b) => {
    const av = sortValue(a, sort.key);
    const bv = sortValue(b, sort.key);
    let cmp: number;
    if (typeof av === "number" && typeof bv === "number") cmp = av - bv;
    else cmp = String(av).localeCompare(String(bv));
    return cmp !== 0 ? sign * cmp : a.seq - b.seq;
  });
}

/** One point of the risk/utilisation-against-band series. */
export interface UtilizationPoint {
  seq: number;
  atNanos: number;
  utilization: number;
  band: string;
  /** Whether the decision at this point actually acted. */
  acted: boolean;
}

/**
 * The utilisation-against-band series for one `(book, instrument)` cell, oldest first.
 *
 * Built ONLY from recorded hedge rows — this is the portfolio state that produced each
 * decision, not a re-derived risk curve. Rows from other engines carry no band and are
 * excluded rather than plotted at zero.
 */
export function utilizationSeries(
  rows: readonly DecisionRecord[],
  book: string,
  instrument: string,
): UtilizationPoint[] {
  return rows
    .filter((r) => r.engine === "hedge" && r.book === book && r.instrument === instrument)
    .map((r) => ({
      seq: r.seq,
      atNanos: r.decidedAtNanos,
      utilization: r.utilization,
      band: r.band,
      acted: r.outcome === "fired",
    }))
    .sort((a, b) => a.seq - b.seq);
}

/** One resolved step of a walked path. */
export interface WalkedStep {
  /** The node id the server recorded. */
  id: number;
  /** A human summary of that node in the CURRENT graph, or `null` when it is gone. */
  label: string | null;
  /** True when the current graph no longer contains this node id. */
  stale: boolean;
  /** True for the last step — the leaf that decided. */
  leaf: boolean;
}

function hedgeNodeSummary(node: HedgeNode): string {
  if (node.kind === "action") return `Action: ${node.action.kind}`;
  const c = node.condition;
  return `${fieldLabel(c.field)} ${opGlyph(c.op)} ${valueLabel(c.value)}`;
}

function acceptanceNodeSummary(node: AcceptanceNode): string {
  if (node.kind === "decision") return `Decision: ${acceptanceActionLabel(node.action.kind)}`;
  const c = node.condition;
  return `${fieldLabel(c.field)} ${opGlyph(c.op)} ${valueLabel(c.value)}`;
}

/**
 * Resolve a recorded `policyPath` against the graph as it stands NOW.
 *
 * A node the current graph no longer contains is reported `stale` with a `null` label —
 * never re-labelled from a neighbouring node and never silently dropped. That is the
 * difference between "your rule 3 held this risk" and a fabricated audit trail.
 */
export function resolveWalkedPath(
  path: readonly number[],
  graph: HedgeGraph | AcceptanceGraph | null,
  engine: DecisionEngine,
): WalkedStep[] {
  const byId = new Map<number, HedgeNode | AcceptanceNode>();
  if (graph !== null) {
    for (const n of graph.nodes) byId.set(n.id, n);
  }
  return path.map((id, i) => {
    const node = byId.get(id);
    if (node === undefined) {
      return { id, label: null, stale: true, leaf: i === path.length - 1 };
    }
    const label =
      engine === "acceptance"
        ? acceptanceNodeSummary(node as AcceptanceNode)
        : hedgeNodeSummary(node as HedgeNode);
    return { id, label, stale: false, leaf: i === path.length - 1 };
  });
}

/**
 * The one-line explanation of what the audit window covers. Returns `null` when the
 * window is complete; a warning sentence when rows have been evicted.
 *
 * The surface MUST render this when present: a bounded ring that has rolled is not a
 * complete audit log and must not be presented as one.
 */
export function windowCaveat(totalRecorded: number, evicted: number): string | null {
  if (evicted <= 0) return null;
  return `This window is INCOMPLETE: ${evicted.toLocaleString()} of ${totalRecorded.toLocaleString()} recorded decisions have already rolled off the server's bounded journal. Older decisions cannot be recovered.`;
}
