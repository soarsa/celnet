/**
 * Risk-transfer mock-flow tests — the offline transport is a GENUINE in-memory store
 * with real risk math (not a stub), so the whole ticket → inbox → audit lifecycle is
 * exercisable end-to-end with no server:
 *
 *   • a RE_ATTRIBUTE within one desk books immediately (BOOKED) and moves a signed risk
 *     slice OUT of the source and INTO the target (visible in `listRiskBookRisk`);
 *   • a DESK_TO_DESK transfer lands PENDING and surfaces on `streamRiskTransferInbox`;
 *   • accepting it books the legs, moves the risk, and appends an audit row with
 *     provenance to `listRiskTransfers`.
 */
import { describe, expect, it } from "vitest";

import { createMockTransport } from "../src/data/mockSource";
import type { CelnetTransport } from "../src/data/transport";
import type { RiskBookRisk, RiskTransfer, TransferLeg } from "../src/data/contract";

async function signedInAdmin(): Promise<CelnetTransport> {
  const t = createMockTransport();
  const res = await t.login("admin@celnet.com", "password");
  t.setSessionToken(res.token);
  return t;
}

function netOf(rows: readonly RiskBookRisk[], bookId: string): number {
  return rows.find((r) => r.bookId === bookId)?.netNotional ?? 0;
}

function leg(riskBookId: string, deskId: string, positionIds: bigint[] = []): TransferLeg {
  return { riskBookId, deskId, trader: "", positionIds };
}

describe("risk-transfer mock — RE_ATTRIBUTE books immediately + moves risk", () => {
  it("moves a signed slice out of the source and into the same-desk target", async () => {
    const t = await signedInAdmin();
    const before = await t.listRiskBookRisk();
    const srcBefore = netOf(before, "fx-emea");
    const tgtBefore = netOf(before, "fx-emea-vanilla");

    const result = await t.initiateRiskTransfer({
      kind: "RE_ATTRIBUTE",
      source: leg("fx-emea", "emea", [101n, 102n]),
      target: leg("fx-emea-vanilla", "emea"),
      quantityFull: true,
      partialNotional: null,
      priceBasis: "MID",
      agreedPrice: null,
      reason: "",
    });

    expect(result.state).toBe("BOOKED");
    expect(result.approver).toBeNull(); // single-control (same desk)
    expect(result.provenance).not.toBeNull();

    const after = await t.listRiskBookRisk();
    // Source net drops by the moved slice; target net rises by it.
    expect(Math.abs(netOf(after, "fx-emea"))).toBeLessThan(Math.abs(srcBefore) + 1e-6);
    expect(netOf(after, "fx-emea")).not.toBe(srcBefore);
    expect(netOf(after, "fx-emea-vanilla")).not.toBe(tgtBefore);
    // Conservation: what left the source arrived at the target (net firm-flat move).
    const moved = result.provenance!.riskMoved.notionalBase;
    expect(netOf(after, "fx-emea")).toBeCloseTo(srcBefore - moved, 3);
    expect(netOf(after, "fx-emea-vanilla")).toBeCloseTo(tgtBefore + moved, 3);
  });

  it("rejects a re-attribution that crosses desks", async () => {
    const t = await signedInAdmin();
    await expect(
      t.initiateRiskTransfer({
        kind: "RE_ATTRIBUTE",
        source: leg("fi-rates-emea", "emea"),
        target: leg("fi-marex", "marex"),
        quantityFull: true,
        partialNotional: null,
        priceBasis: "MID",
        agreedPrice: null,
        reason: "",
      }),
    ).rejects.toThrow(/one desk/i);
  });
});

describe("risk-transfer mock — DESK_TO_DESK: pending → inbox → accept → booked + audited", () => {
  it("drives the full four-eyes lifecycle end to end", async () => {
    const t = await signedInAdmin();
    let inbox: RiskTransfer[] = [];
    const dispose = t.streamRiskTransferInbox((p) => {
      inbox = p;
    });
    expect(inbox).toEqual([]); // baseline: nothing pending

    const before = await t.listRiskBookRisk();
    const srcBefore = netOf(before, "fi-rates-emea");
    const tgtBefore = netOf(before, "fi-marex");

    const pending = await t.initiateRiskTransfer({
      kind: "DESK_TO_DESK",
      source: leg("fi-rates-emea", "emea", [201n]),
      target: leg("fi-marex", "marex"),
      quantityFull: true,
      partialNotional: null,
      priceBasis: "MARK_TO_MARKET",
      agreedPrice: null,
      reason: "hedge hand-over",
    });
    expect(pending.state).toBe("PENDING");
    // The inbox push carried the new pending transfer.
    expect(inbox.map((x) => x.id)).toContain(pending.id);

    const booked = await t.acceptRiskTransfer(pending.id);
    expect(booked.state).toBe("BOOKED");
    expect(booked.approver).not.toBeNull();
    expect(booked.provenance).not.toBeNull();
    // Accepting drains it from the pending inbox.
    expect(inbox.map((x) => x.id)).not.toContain(pending.id);

    const after = await t.listRiskBookRisk();
    const moved = booked.provenance!.riskMoved.notionalBase;
    expect(netOf(after, "fi-rates-emea")).toBeCloseTo(srcBefore - moved, 3);
    expect(netOf(after, "fi-marex")).toBeCloseTo(tgtBefore + moved, 3);

    // The audit trail carries the booked record, newest first, with provenance.
    const audit = await t.listRiskTransfers({ desk: null, trader: null, riskBookId: null, states: [] });
    const row = audit.find((x) => x.id === booked.id);
    expect(row).toBeDefined();
    expect(row!.state).toBe("BOOKED");
    expect(row!.provenance?.approver).toBe(booked.approver);

    // The state filter narrows to booked only.
    const bookedOnly = await t.listRiskTransfers({ desk: null, trader: null, riskBookId: null, states: ["BOOKED"] });
    expect(bookedOnly.every((x) => x.state === "BOOKED")).toBe(true);
    expect(bookedOnly.some((x) => x.id === booked.id)).toBe(true);

    dispose();
  });

  it("rejecting a pending transfer removes it from the inbox and does not move risk", async () => {
    const t = await signedInAdmin();
    let inbox: RiskTransfer[] = [];
    const dispose = t.streamRiskTransferInbox((p) => {
      inbox = p;
    });
    const before = await t.listRiskBookRisk();

    const pending = await t.initiateRiskTransfer({
      kind: "DESK_TO_DESK",
      source: leg("fi-rates-emea", "emea", [301n]),
      target: leg("fi-marex", "marex"),
      quantityFull: false,
      partialNotional: 10_000_000,
      priceBasis: "MID",
      agreedPrice: null,
      reason: "",
    });
    expect(inbox.map((x) => x.id)).toContain(pending.id);

    const rejected = await t.rejectRiskTransfer(pending.id, "not this desk");
    expect(rejected.state).toBe("REJECTED");
    expect(inbox.map((x) => x.id)).not.toContain(pending.id);

    // A rejected transfer moves no risk — nets are unchanged.
    const after = await t.listRiskBookRisk();
    expect(netOf(after, "fi-rates-emea")).toBeCloseTo(netOf(before, "fi-rates-emea"), 6);
    expect(netOf(after, "fi-marex")).toBeCloseTo(netOf(before, "fi-marex"), 6);

    dispose();
  });
});
