/**
 * The ticket task-pane state machines — pure, framework-free, unit-testable.
 *
 * The task pane drives two write-path workflows (docs §3.3 / §4): an RFQ ticket
 * (request a two-way, then click-to-trade a live token) and a mark-surface
 * contribution (stage, then confirm). Both are explicit pending → confirmed /
 * rejected state machines because contribution and trading are entitlement-gated,
 * audited, and never a silent side effect (docs §1, the read/write split). This
 * module holds the pure transitions; `taskpane.ts` binds them to the DOM and the
 * transport. Keeping the transitions pure lets the node unit suite exercise every
 * state path with no Office host and no server.
 */

export type TicketPhase = "idle" | "pending" | "confirmed" | "rejected";

/** The RFQ ticket state: a quoted two-way and the trade lifecycle on it. */
export interface RfqState {
  readonly phase: TicketPhase;
  readonly bid: number;
  readonly offer: number;
  readonly quoteId: bigint;
  readonly validUntilNanos: bigint;
  /** A human-readable status line shown under the ticket. */
  readonly status: string;
  /** True once a live, in-window quote is tradable (enables the Trade buttons). */
  readonly tradable: boolean;
}

export const IDLE_RFQ: RfqState = {
  phase: "idle",
  bid: 0,
  offer: 0,
  quoteId: 0n,
  validUntilNanos: 0n,
  status: "",
  tradable: false,
};

/** A decoded quote the RFQ ticket transitions on. */
export interface QuotedTwoWay {
  readonly bid: number;
  readonly offer: number;
  readonly quoteId: bigint;
  readonly validUntilNanos: bigint;
}

/** Transition the RFQ ticket to a live quote (pending acceptance). */
export function rfqQuoted(q: QuotedTwoWay): RfqState {
  return {
    phase: "pending",
    bid: q.bid,
    offer: q.offer,
    quoteId: q.quoteId,
    validUntilNanos: q.validUntilNanos,
    status: `quoted ${q.bid.toFixed(5)} / ${q.offer.toFixed(5)} — id ${q.quoteId.toString()}`,
    tradable: q.bid > 0 && q.offer > 0 && q.offer >= q.bid,
  };
}

/** Transition to confirmed after an Execution books (the trade is done). */
export function rfqExecuted(prev: RfqState, side: "BUY" | "SELL", tradedPremium: number): RfqState {
  return {
    ...prev,
    phase: "confirmed",
    tradable: false,
    status: `EXECUTED ${side} @ ${tradedPremium.toFixed(5)}`,
  };
}

/** Transition to rejected (a stale/forged/already-consumed token, or a denial). */
export function rfqRejected(prev: RfqState, reason: string): RfqState {
  return { ...prev, phase: "rejected", tradable: false, status: `REJECTED — ${reason}` };
}

// ---------------------------------------------------------------------------
// mark-surface contribution ticket
// ---------------------------------------------------------------------------

export type MarkPhase = "idle" | "pending" | "committed" | "rejected";

export interface MarkState {
  readonly phase: MarkPhase;
  readonly stagingId: string;
  readonly status: string;
  /** The resulting surface_version after a committed contribution, if known. */
  readonly surfaceVersionAfter?: bigint;
}

export const IDLE_MARK: MarkState = { phase: "idle", stagingId: "", status: "" };

/** A contribution has been staged (PENDING) and awaits confirmation. */
export function markStaged(stagingId: string): MarkState {
  return { phase: "pending", stagingId, status: "staged — confirm to contribute" };
}

/** The contribution committed; record the resulting surface_version + audit id. */
export function markCommittedState(prev: MarkState, surfaceVersionAfter: bigint, auditId: string): MarkState {
  return {
    ...prev,
    phase: "committed",
    surfaceVersionAfter,
    status: `COMMITTED — surface v${surfaceVersionAfter} · audit ${auditId}`,
  };
}

/** The contribution was rejected on the server (e.g. convention mismatch). */
export function markRejectedState(prev: MarkState, reason: string): MarkState {
  return { ...prev, phase: "rejected", status: `REJECTED — ${reason}` };
}

/** True iff a staged contribution can be confirmed (it is pending). */
export function canCommitMark(state: MarkState): boolean {
  return state.phase === "pending" && state.stagingId.length > 0;
}
