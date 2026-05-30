import { afterEach, describe, expect, it } from "vitest";
import {
  clearStaging,
  getStaged,
  idempotencyKey,
  markCommitted,
  pendingMarks,
  stageMark,
} from "../src/functions/markStaging";
import { ShapingError } from "../src/functions/shaping";
import type { Connection } from "../src/transport/connection";

// stageMark never touches the connection (two-phase: stage now, commit later), so
// a bare cast is sufficient — there is nothing of OUR functionality to mock.
const NO_CONN = {} as unknown as Connection;

afterEach(() => clearStaging());

describe("two-phase idempotent mark staging", () => {
  it("a recalc with unchanged args re-stages under the SAME key (no double-mark)", async () => {
    const args = { pair: "EURUSD", tenor: "1Y", pillar: "ATM", vol: 0.102, comment: "" };
    const first = await stageMark(NO_CONN, args);
    const second = await stageMark(NO_CONN, args);
    expect(first.stagingId).toBe(second.stagingId);
    expect(first.status).toBe("PENDING");
    expect(pendingMarks().length).toBe(1); // one pending, not two
  });

  it("distinct contributions stage under distinct keys", async () => {
    await stageMark(NO_CONN, { pair: "EURUSD", tenor: "1Y", pillar: "ATM", vol: 0.102, comment: "" });
    await stageMark(NO_CONN, { pair: "EURUSD", tenor: "1Y", pillar: "ATM", vol: 0.103, comment: "" });
    expect(pendingMarks().length).toBe(2);
  });

  it("never auto-commits: staging leaves PENDING until an explicit commit", async () => {
    const s = await stageMark(NO_CONN, { pair: "EURUSD", tenor: "1Y", pillar: "25dP", vol: 0.108, comment: "" });
    expect(getStaged(s.stagingId)?.status).toBe("PENDING");
    markCommitted(s.stagingId, 1300n, "audit-1");
    expect(getStaged(s.stagingId)?.status).toBe("COMMITTED");
    expect(getStaged(s.stagingId)?.surfaceVersionAfter).toBe(1300n);
    expect(pendingMarks().length).toBe(0); // committed leaves the pending set
  });

  it("validates the contribution shape against the contract vocabulary", async () => {
    await expect(stageMark(NO_CONN, { pair: "EUR", tenor: "1Y", pillar: "ATM", vol: 0.1, comment: "" })).rejects.toThrow(ShapingError);
    await expect(stageMark(NO_CONN, { pair: "EURUSD", tenor: "1X", pillar: "ATM", vol: 0.1, comment: "" })).rejects.toThrow(ShapingError);
    await expect(stageMark(NO_CONN, { pair: "EURUSD", tenor: "1Y", pillar: "ATM", vol: 0, comment: "" })).rejects.toThrow(ShapingError);
  });

  it("the idempotency key is deterministic for identical args", () => {
    const a = idempotencyKey({ pair: "EURUSD", tenor: "1Y", pillar: "ATM", vol: 0.1, comment: "x" });
    const b = idempotencyKey({ pair: "eurusd", tenor: "1y", pillar: "atm", vol: 0.1, comment: "DIFFERENT" });
    // Comment is not part of the identity; case/whitespace normalized.
    expect(a).toBe(b);
  });
});
