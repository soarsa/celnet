import { describe, expect, it } from "vitest";
import {
  IDLE_MARK,
  IDLE_RFQ,
  canCommitMark,
  markCommittedState,
  markRejectedState,
  markStaged,
  rfqExecuted,
  rfqQuoted,
  rfqRejected,
} from "../src/taskpane/ticketModel";

describe("RFQ ticket state machine", () => {
  it("idle -> quoted (pending, tradable) -> executed (confirmed)", () => {
    let s = IDLE_RFQ;
    expect(s.phase).toBe("idle");
    s = rfqQuoted({ bid: 0.039, offer: 0.041, quoteId: 7n, validUntilNanos: 123n });
    expect(s.phase).toBe("pending");
    expect(s.tradable).toBe(true);
    expect(s.status).toContain("id 7");
    s = rfqExecuted(s, "BUY", 0.041);
    expect(s.phase).toBe("confirmed");
    expect(s.tradable).toBe(false);
    expect(s.status).toContain("EXECUTED BUY");
  });

  it("a degenerate two-way is not tradable", () => {
    const s = rfqQuoted({ bid: 0, offer: 0, quoteId: 1n, validUntilNanos: 0n });
    expect(s.tradable).toBe(false);
  });

  it("quoted -> rejected on a stale/forged token", () => {
    const s = rfqRejected(rfqQuoted({ bid: 0.039, offer: 0.041, quoteId: 9n, validUntilNanos: 1n }), "ALREADY_CONSUMED");
    expect(s.phase).toBe("rejected");
    expect(s.tradable).toBe(false);
    expect(s.status).toContain("ALREADY_CONSUMED");
  });
});

describe("mark contribution state machine", () => {
  it("idle -> staged (pending, commitable) -> committed", () => {
    let m = IDLE_MARK;
    expect(canCommitMark(m)).toBe(false);
    m = markStaged("mark-abc");
    expect(m.phase).toBe("pending");
    expect(canCommitMark(m)).toBe(true);
    m = markCommittedState(m, 1285n, "audit-42");
    expect(m.phase).toBe("committed");
    expect(m.surfaceVersionAfter).toBe(1285n);
    expect(m.status).toContain("surface v1285");
    expect(canCommitMark(m)).toBe(false);
  });

  it("staged -> rejected on a convention mismatch", () => {
    const m = markRejectedState(markStaged("mark-x"), "declared 25Δ premium-adj vs resolved spot");
    expect(m.phase).toBe("rejected");
    expect(m.status).toContain("REJECTED");
    expect(canCommitMark(m)).toBe(false);
  });
});
