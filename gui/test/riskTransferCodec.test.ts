/**
 * Risk-transfer wsCodec round-trip + wire-shape tests — the GUI end of the ONE
 * `celnet.wire` risk-transfer contract (server phase-2). Enum tags map to their proto
 * ordinals and back; a `TransferLeg` round-trips `fromWire(toWire(x)) === x`; the five
 * request framers produce the exact snake_case + NUMERIC-enum JSON the server speaks
 * (optionals OMITTED when absent); the response/inbox decoders reconstruct the typed
 * record (source/target legs, flat quantity/price sum-types, provenance optionals, and
 * bigint position ids without f64 rounding). No server, no mocks — the REAL codec.
 */
import { describe, expect, it } from "vitest";

import { transferKind, transferState, transferPriceBasis, priceBasis } from "../src/data/enums";
import {
  transferLegToWire,
  transferLegFromWire,
  riskTransferFromWire,
  riskTransferProvenanceFromWire,
  initiateRiskTransferRequestToWire,
  acceptRiskTransferRequestToWire,
  rejectRiskTransferRequestToWire,
  cancelRiskTransferRequestToWire,
  listRiskTransfersRequestToWire,
  riskTransferResponseFromWire,
  listRiskTransfersResponseFromWire,
  riskTransferInboxFromWire,
} from "../src/data/wsCodec";
import type { TransferKind, TransferLeg, TransferState } from "../src/data/contract";

describe("risk-transfer codec — enum ordinal maps", () => {
  it("maps TransferKind to proto ordinals 1..3 and back", () => {
    const kinds: TransferKind[] = ["RE_ATTRIBUTE", "DESK_TO_DESK", "TRADER_TO_TRADER"];
    kinds.forEach((k, i) => {
      expect(transferKind.toWire(k)).toBe(i + 1);
      expect(transferKind.fromWire(i + 1)).toBe(k);
    });
  });

  it("maps TransferState to proto ordinals 1..6 and back", () => {
    const states: TransferState[] = ["DRAFT", "PENDING", "ACCEPTED", "REJECTED", "BOOKED", "CANCELLED"];
    states.forEach((s, i) => {
      expect(transferState.toWire(s)).toBe(i + 1);
      expect(transferState.fromWire(i + 1)).toBe(s);
    });
  });

  it("maps TransferPriceBasis + PriceBasis to ordinals 1..3 and back", () => {
    (["MID", "MARK_TO_MARKET", "AGREED"] as const).forEach((b, i) => {
      expect(transferPriceBasis.toWire(b)).toBe(i + 1);
      expect(transferPriceBasis.fromWire(i + 1)).toBe(b);
      expect(priceBasis.toWire(b)).toBe(i + 1);
      expect(priceBasis.fromWire(i + 1)).toBe(b);
    });
  });
});

describe("risk-transfer codec — TransferLeg round-trip + wire shape", () => {
  it("round-trips a leg with bigint position ids (no f64 rounding)", () => {
    const leg: TransferLeg = {
      riskBookId: "fi-rates-emea",
      deskId: "emea",
      trader: "fi.trader@celnet.com",
      positionIds: [9007199254740993n, 1n, 6817263741029384756n],
    };
    const wire = transferLegToWire(leg);
    expect(wire).toMatchObject({
      risk_book_id: "fi-rates-emea",
      desk_id: "emea",
      trader: "fi.trader@celnet.com",
    });
    expect(transferLegFromWire(wire)).toEqual(leg);
  });
});

describe("risk-transfer codec — request framers (snake_case + numeric enums, omit-absent)", () => {
  it("frames a Full MID initiate with the price/quantity optionals OMITTED", () => {
    const wire = initiateRiskTransferRequestToWire({
      kind: "DESK_TO_DESK",
      source: { riskBookId: "fi-rates-emea", deskId: "emea", trader: "a@x", positionIds: [1n] },
      target: { riskBookId: "fi-marex", deskId: "marex", trader: "", positionIds: [] },
      quantityFull: true,
      partialNotional: null,
      priceBasis: "MID",
      agreedPrice: null,
      reason: "",
    });
    expect(wire.kind).toBe(2);
    expect(wire.price_basis).toBe(1);
    expect(wire.quantity_full).toBe(true);
    expect("partial_notional" in wire).toBe(false);
    expect("agreed_price" in wire).toBe(false);
  });

  it("frames a Partial AGREED initiate carrying both override optionals", () => {
    const wire = initiateRiskTransferRequestToWire({
      kind: "RE_ATTRIBUTE",
      source: { riskBookId: "fx-emea", deskId: "emea", trader: "a@x", positionIds: [1n, 2n] },
      target: { riskBookId: "fx-emea-vanilla", deskId: "emea", trader: "", positionIds: [] },
      quantityFull: false,
      partialNotional: 250_000_000,
      priceBasis: "AGREED",
      agreedPrice: 99.5,
      reason: "off-mark cross agreed with control",
    });
    expect(wire.kind).toBe(1);
    expect(wire.price_basis).toBe(3);
    expect(wire.partial_notional).toBe(250_000_000);
    expect(wire.agreed_price).toBe(99.5);
    expect(wire.reason).toBe("off-mark cross agreed with control");
  });

  it("frames accept / reject / cancel by transfer id", () => {
    expect(acceptRiskTransferRequestToWire("xfer-3")).toEqual({ transfer_id: "xfer-3" });
    expect(rejectRiskTransferRequestToWire("xfer-3", "no")).toEqual({ transfer_id: "xfer-3", reason: "no" });
    expect(cancelRiskTransferRequestToWire("xfer-3")).toEqual({ transfer_id: "xfer-3" });
  });

  it("frames a list filter, omitting each absent axis + encoding states as tags", () => {
    expect(
      listRiskTransfersRequestToWire({ desk: null, trader: null, riskBookId: null, states: [] }),
    ).toEqual({});
    const wire = listRiskTransfersRequestToWire({
      desk: "emea",
      trader: null,
      riskBookId: "fi-marex",
      states: ["PENDING", "BOOKED"],
    });
    expect(wire).toEqual({ desk: "emea", risk_book_id: "fi-marex", states: [2, 5] });
  });
});

describe("risk-transfer codec — response + inbox decoders", () => {
  const bookedWire = {
    id: "xfer-7",
    kind: 2,
    source: { risk_book_id: "fi-rates-emea", desk_id: "emea", trader: "a@x", position_ids: [11n, 12n] },
    target: { risk_book_id: "fi-marex", desk_id: "marex", trader: "b@y", position_ids: [] },
    quantity_full: true,
    price_basis: 1,
    reason: "",
    initiated_by: "a@x",
    initiated_at: 1_700_000_000_000_000_000n,
    state: 5,
    approver: "b@y",
    decided_at: 1_700_000_000_500_000_000n,
    transfer_price: 100,
    provenance: {
      transfer_id: "xfer-7",
      kind: 2,
      initiated_by: "a@x",
      initiated_at: 1_700_000_000_000_000_000n,
      approver: "b@y",
      decided_at: 1_700_000_000_500_000_000n,
      source_book_id: "fi-rates-emea",
      target_book_id: "fi-marex",
      position_ids: [11n, 12n],
      quantity_full: true,
      transfer_price: 100,
      price_basis: 1,
      reason: "",
      realized_pnl_source: 0,
      risk_moved: { notional_base: 5_000_000, risk: { dv01: 12, delta: 0, gamma: 0, vega: 0, theta: 0 } },
    },
  };

  it("decodes a booked transfer response into the typed record", () => {
    const t = riskTransferResponseFromWire({ transfer: bookedWire });
    expect(t.id).toBe("xfer-7");
    expect(t.kind).toBe("DESK_TO_DESK");
    expect(t.state).toBe("BOOKED");
    expect(t.priceBasis).toBe("MID");
    expect(t.quantityFull).toBe(true);
    expect(t.partialNotional).toBeNull();
    expect(t.transferPrice).toBe(100);
    expect(t.source.positionIds).toEqual([11n, 12n]);
    expect(t.approver).toBe("b@y");
    expect(t.provenance?.riskMoved.notionalBase).toBe(5_000_000);
    expect(t.provenance?.riskMoved.risk.dv01).toBe(12);
  });

  it("decodes a pending transfer with no provenance (null) via riskTransferFromWire", () => {
    const t = riskTransferFromWire({
      id: "xfer-8",
      kind: 3,
      source: { risk_book_id: "fx-emea", desk_id: "emea", trader: "a@x", position_ids: [1n] },
      target: { risk_book_id: "fx-apac", desk_id: "apac", trader: "", position_ids: [] },
      quantity_full: false,
      partial_notional: 42,
      price_basis: 3,
      agreed_price: 98.75,
      reason: "hand off",
      initiated_by: "a@x",
      initiated_at: 5n,
      state: 2,
    });
    expect(t.kind).toBe("TRADER_TO_TRADER");
    expect(t.state).toBe("PENDING");
    expect(t.partialNotional).toBe(42);
    expect(t.agreedPrice).toBe(98.75);
    expect(t.approver).toBeNull();
    expect(t.transferPrice).toBeNull();
    expect(t.provenance).toBeNull();
  });

  it("decodes the list reply + the inbox push frame", () => {
    const list = listRiskTransfersResponseFromWire({ transfers: [bookedWire] });
    expect(list).toHaveLength(1);
    expect(list[0]!.id).toBe("xfer-7");

    const inbox = riskTransferInboxFromWire({ pending: [bookedWire], at_nanos: 1_700_000_000_999_000_000n });
    expect(inbox.pending).toHaveLength(1);
    expect(inbox.atNanos).toBe(1_700_000_000_999_000_000n);
  });

  it("decodes a provenance block standalone", () => {
    const p = riskTransferProvenanceFromWire(bookedWire.provenance);
    expect(p.transferId).toBe("xfer-7");
    expect(p.priceBasis).toBe("MID");
    expect(p.riskMoved.risk.dv01).toBe(12);
  });
});
