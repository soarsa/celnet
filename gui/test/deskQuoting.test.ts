import { describe, expect, it } from "vitest";

import type {
  DeskRequest,
  Notification,
  OisInstrument,
  RatesCurveSet,
  SubmitDeskRequestRequest,
} from "../src/data/contract";
import { createMockTransport } from "../src/data/mockSource";
import { DEFAULT_USD_SOFR_CURVE } from "../src/data/ratesPricing";
import * as e from "../src/data/enums";
import {
  acceptDeskQuoteToWire,
  bookRatesPositionToWire,
  dealFromWire,
  deskRequestFromWire,
  listDeskRequestsToWire,
  notificationFromWire,
  ratesPositionFromWire,
  respondDeskRequestToWire,
  submitDeskRequestToWire,
  subscribeNotificationsToWire,
  type WireObject,
} from "../src/data/wsCodec";

/**
 * Behavioural tests for the dealer-quoting desk + rates Book + notification push.
 * The offline `MockTransport` is the FULL real implementation (no stub), so the
 * lifecycle invariants the live `RfqDeskService` / `NotificationService` edge must
 * satisfy — submit→quote→accept books a deal AND a rates position, reject is
 * terminal, every transition pushes the right notification — are asserted here,
 * plus codec round-trips that pin the exact snake_case + numeric-enum wire shape
 * the server codec (`crates/celnet-server/src/ws/codec.rs`) decodes/encodes.
 */

const CURVE: RatesCurveSet = DEFAULT_USD_SOFR_CURVE;

function ois(overrides: Partial<OisInstrument> = {}): OisInstrument {
  return {
    tenorYears: 5,
    fixedRate: 0.04,
    notional: 50_000_000,
    direction: "PAY_FIXED",
    ...overrides,
  };
}

function submitReq(
  overrides: Partial<SubmitDeskRequestRequest> = {},
): SubmitDeskRequestRequest {
  return {
    kind: "RFQ",
    counterparty: "Acme Capital",
    desk: "g10-rates",
    instrument: ois(),
    curveSet: CURVE,
    side: "BUY",
    notional: 50_000_000,
    ttlMs: 60_000,
    ...overrides,
  };
}

describe("desk quoting — offline lifecycle", () => {
  it("submits a PENDING request and pushes a *_RECEIVED notification", async () => {
    const t = createMockTransport();
    const seen: Notification[] = [];
    const dispose = t.streamNotifications(undefined, (n) => seen.push(n));

    const submitted = await t.submitDeskRequest(submitReq({ kind: "IOI" }));
    expect(submitted.request.state).toBe("PENDING");
    expect(submitted.request.kind).toBe("IOI");

    const got = seen.find((n) => n.requestId === submitted.request.requestId);
    expect(got?.kind).toBe("IOI_RECEIVED");
    expect(got?.requestKind).toBe("IOI");
    dispose();
  });

  it("quotes a request (→ QUOTED) then accepts it, booking a deal AND a position", async () => {
    const t = createMockTransport();
    const seen: Notification[] = [];
    t.streamNotifications(undefined, (n) => seen.push(n));

    const { request } = await t.submitDeskRequest(submitReq());
    const positionsBefore = (await t.listRatesPositions({})).positions.length;

    const quoted = await t.respondDeskRequest({
      requestId: request.requestId,
      response: {
        kind: "quote",
        quote: {
          price: 0.0415,
          notional: 50_000_000,
          validForMs: 30_000,
          trader: "Robin",
        },
      },
    });
    expect(quoted.request.state).toBe("QUOTED");
    expect(quoted.request.quote?.price).toBeCloseTo(0.0415, 10);

    const accepted = await t.acceptDeskQuote({ requestId: request.requestId });
    expect(accepted.request.state).toBe("ACCEPTED");
    expect(accepted.deal.price).toBeCloseTo(0.0415, 10);
    expect(accepted.deal.requestId).toBe(request.requestId);
    expect(accepted.deal.positionId).toBeDefined();

    // The deal lands in the blotter and the booked position in the rates book.
    const deals = await t.listDeals({});
    expect(deals.deals.some((d) => d.dealId === accepted.deal.dealId)).toBe(
      true,
    );
    const positions = await t.listRatesPositions({});
    expect(positions.positions.length).toBe(positionsBefore + 1);
    expect(
      positions.positions.some(
        (p) => p.positionId === accepted.deal.positionId,
      ),
    ).toBe(true);

    // Accept pushed a QUOTE_ACCEPTED notification.
    expect(
      seen.some(
        (n) => n.kind === "QUOTE_ACCEPTED" && n.requestId === request.requestId,
      ),
    ).toBe(true);
  });

  it("rejects a request (→ REJECTED), pushes QUOTE_REJECTED, and blocks accept", async () => {
    const t = createMockTransport();
    const seen: Notification[] = [];
    t.streamNotifications(undefined, (n) => seen.push(n));

    const { request } = await t.submitDeskRequest(submitReq());
    const rejected = await t.respondDeskRequest({
      requestId: request.requestId,
      response: { kind: "reject", reject: { reason: "axe filled" } },
    });
    expect(rejected.request.state).toBe("REJECTED");
    expect(
      seen.some(
        (n) => n.kind === "QUOTE_REJECTED" && n.requestId === request.requestId,
      ),
    ).toBe(true);

    await expect(
      t.acceptDeskQuote({ requestId: request.requestId }),
    ).rejects.toThrow();
  });

  it("filters the inbox by the request-state scope", async () => {
    const t = createMockTransport();
    const { request } = await t.submitDeskRequest(submitReq());
    await t.respondDeskRequest({
      requestId: request.requestId,
      response: {
        kind: "quote",
        quote: {
          price: 0.04,
          notional: 10_000_000,
          validForMs: 1000,
          trader: "T",
        },
      },
    });
    const quotedOnly = await t.listDeskRequests({
      scope: { states: ["QUOTED"] },
    });
    expect(quotedOnly.requests.length).toBeGreaterThan(0);
    expect(quotedOnly.requests.every((r) => r.state === "QUOTED")).toBe(true);
  });

  it("does not deliver a scoped notification to a mismatched desk subscriber", async () => {
    const t = createMockTransport();
    const seen: Notification[] = [];
    t.streamNotifications({ desks: ["fx-desk"] }, (n) => seen.push(n));
    await t.submitDeskRequest(submitReq({ desk: "g10-rates" }));
    expect(seen).toHaveLength(0);
  });
});

describe("rates Book — offline", () => {
  it("books a position with a minted id and lists it", async () => {
    const t = createMockTransport();
    const before = (await t.listRatesPositions({})).positions.length;
    const booked = await t.bookRatesPosition({
      position: {
        positionId: 0n,
        entity: 7,
        book: 70,
        instrument: ois({ tenorYears: 3 }),
      },
    });
    expect(booked.position.positionId).toBeGreaterThan(0n);
    const after = await t.listRatesPositions({});
    expect(after.positions.length).toBe(before + 1);
    expect(
      after.positions.some((p) => p.positionId === booked.position.positionId),
    ).toBe(true);
  });

  it("narrows the listed positions by the book scope", async () => {
    const t = createMockTransport();
    await t.bookRatesPosition({
      position: { positionId: 0n, entity: 9, book: 91, instrument: ois() },
    });
    const scoped = await t.listRatesPositions({ scope: { book: 91 } });
    expect(scoped.positions.length).toBeGreaterThan(0);
    expect(scoped.positions.every((p) => p.book === 91)).toBe(true);
  });
});

describe("desk quoting — codec round-trips (wire shape)", () => {
  it("encodes SubmitDeskRequest with snake_case fields + numeric enum tags", () => {
    const w = submitDeskRequestToWire(submitReq({ kind: "IOI", side: "SELL" }));
    expect(w["kind"]).toBe(e.deskRequestKind.toWire("IOI")); // 2
    expect(w["side"]).toBe(e.side.toWire("SELL")); // 1
    expect(w["ttl_ms"]).toBe(60_000);
    expect(w["curve_set"]).toBeTypeOf("object");
    expect((w["instrument"] as WireObject)["ois"]).toBeTypeOf("object");
    // The audited explicit grant-all principal clears deny-by-default.
    expect((w["principal"] as WireObject)["grant_all"]).toBe(true);
  });

  it("encodes the respond quote / reject oneof arms exactly", () => {
    const quoteW = respondDeskRequestToWire({
      requestId: "req-1",
      response: {
        kind: "quote",
        quote: { price: 0.04, notional: 1e6, validForMs: 5000, trader: "T" },
      },
    });
    expect(quoteW["request_id"]).toBe("req-1");
    expect((quoteW["quote"] as WireObject)["valid_for_ms"]).toBe(5000);
    expect(quoteW["reject"]).toBeUndefined();

    const rejectW = respondDeskRequestToWire({
      requestId: "req-2",
      response: { kind: "reject", reject: { reason: "no axe" } },
    });
    expect((rejectW["reject"] as WireObject)["reason"]).toBe("no axe");
    expect(rejectW["quote"]).toBeUndefined();
  });

  it("encodes accept / list / book / subscribe bodies", () => {
    expect(acceptDeskQuoteToWire({ requestId: "req-9" })["request_id"]).toBe(
      "req-9",
    );
    const listW = listDeskRequestsToWire({
      scope: { states: ["PENDING", "QUOTED"] },
    });
    expect((listW["scope"] as WireObject)["states"]).toEqual([
      e.deskRequestState.toWire("PENDING"),
      e.deskRequestState.toWire("QUOTED"),
    ]);
    const bookW = bookRatesPositionToWire({
      position: { positionId: 3n, entity: 1, book: 10, instrument: ois() },
    });
    expect((bookW["position"] as WireObject)["position_id"]).toBe(3);
    const subW = subscribeNotificationsToWire({ desks: ["g10-rates"] });
    expect((subW["scope"] as WireObject)["desks"]).toEqual(["g10-rates"]);
  });

  it("decodes a server DeskRequest frame (enum tags → string members)", () => {
    const wire: WireObject = {
      request_id: "req-7",
      kind: 1, // RFQ
      counterparty: "Globex",
      desk: "g10-rates",
      instrument: {
        ois: { tenor_years: 5, fixed_rate: 0.04, notional: 5e7, side: 1 },
      },
      curve_set: {
        currency: "USD",
        reference_date: { year: 2026, month: 6, day: 26 },
        ois_pillars: [{ tenor: { years: 5 }, par_rate: 0.041 }],
      },
      side: 0, // BUY
      notional: 5e7,
      received_at_nanos: 1_700_000_000_000_000_000n,
      expires_at_nanos: 1_700_000_060_000_000_000n,
      state: 2, // QUOTED
      quote: {
        price: 0.0415,
        notional: 5e7,
        valid_for_ms: 30_000,
        trader: "Robin",
      },
    };
    const r: DeskRequest = deskRequestFromWire(wire);
    expect(r.requestId).toBe("req-7");
    expect(r.kind).toBe("RFQ");
    expect(r.side).toBe("BUY");
    expect(r.state).toBe("QUOTED");
    expect(r.instrument.direction).toBe("RECEIVE_FIXED"); // wire side 1 → RECEIVE_FIXED
    expect(r.curveSet.pillars[0]?.parRate).toBeCloseTo(0.041, 10);
    expect(r.quote?.trader).toBe("Robin");
    expect(r.receivedAtNanos).toBe(1_700_000_000_000_000_000n);
  });

  it("decodes a server Deal + RatesPosition + Notification frame", () => {
    const deal = dealFromWire({
      deal_id: "deal-3",
      request_id: "req-7",
      kind: 2, // IOI
      counterparty: "Initech",
      desk: "g10-rates",
      instrument: {
        ois: { tenor_years: 2, fixed_rate: 0.04, notional: 1e7, side: 0 },
      },
      curve_set: {
        currency: "USD",
        reference_date: { year: 2026, month: 6, day: 26 },
        ois_pillars: [],
      },
      side: 1,
      notional: 1e7,
      price: 0.0405,
      executed_at_nanos: 1_700_000_000_000_000_000n,
      trader: "Sam",
      position_id: 42,
    });
    expect(deal.dealId).toBe("deal-3");
    expect(deal.kind).toBe("IOI");
    expect(deal.positionId).toBe(42n);

    const pos = ratesPositionFromWire({
      position_id: 99,
      entity: 1,
      book: 10,
      instrument: {
        ois: { tenor_years: 10, fixed_rate: 0.042, notional: 2.5e7, side: 1 },
      },
    });
    expect(pos.positionId).toBe(99n);
    expect(pos.instrument.direction).toBe("RECEIVE_FIXED");

    const ntf = notificationFromWire({
      type: "notification",
      notification_id: "ntf-5",
      kind: 5, // QUOTE_ACCEPTED
      at_nanos: 1_700_000_000_000_000_000n,
      request_id: "req-7",
      desk: "g10-rates",
      counterparty: "Initech",
      request_kind: 1, // RFQ
      headline: "lifted",
      detail: "deal-3",
    });
    expect(ntf.kind).toBe("QUOTE_ACCEPTED");
    expect(ntf.requestKind).toBe("RFQ");
    expect(ntf.requestId).toBe("req-7");
    expect(ntf.detail).toBe("deal-3");
  });
});

describe("notificationFromWire — server exception contract (alert_worthy + reason)", () => {
  it("defaults alertWorthy to false and omits reason when the fields are absent", () => {
    const ntf = notificationFromWire({
      type: "notification",
      notification_id: "ntf-1",
      kind: 1, // RFQ_RECEIVED
      at_nanos: 1n,
      desk: "g10-rates",
      counterparty: "ACME",
      request_kind: 1,
      headline: "RFQ",
    });
    expect(ntf.alertWorthy).toBe(false);
    expect(ntf.reason).toBeUndefined();
  });

  it("decodes alert_worthy:true", () => {
    const ntf = notificationFromWire({
      type: "notification",
      notification_id: "ntf-2",
      kind: 1,
      at_nanos: 1n,
      desk: "g10-rates",
      counterparty: "ACME",
      request_kind: 1,
      headline: "RFQ",
      alert_worthy: true,
    });
    expect(ntf.alertWorthy).toBe(true);
  });

  it("round-trips a MANUAL_INTERVENTION_REQUIRED frame with its reason ordinal", () => {
    const ntf = notificationFromWire({
      type: "notification",
      notification_id: "ntf-3",
      kind: 7, // MANUAL_INTERVENTION_REQUIRED
      at_nanos: 1n,
      desk: "g10-rates",
      counterparty: "Meridian Capital",
      request_kind: 1,
      headline: "Manual pricing needed",
      detail: "USD-OIS 15Y",
      alert_worthy: true,
      reason: 1, // UNCONFIGURED_TENOR
    });
    expect(ntf.kind).toBe("MANUAL_INTERVENTION_REQUIRED");
    expect(ntf.alertWorthy).toBe(true);
    expect(ntf.reason).toBe("UNCONFIGURED_TENOR");
  });

  it("maps every reason ordinal (1–4) to its enum member", () => {
    const expected: Record<number, string> = {
      1: "UNCONFIGURED_TENOR",
      2: "CREDIT_RISK_BREAK",
      3: "UNKNOWN_SECURITY",
      4: "PRICING_FAILURE",
    };
    for (const [ord, member] of Object.entries(expected)) {
      const ntf = notificationFromWire({
        type: "notification",
        notification_id: `ntf-r${ord}`,
        kind: 7,
        at_nanos: 1n,
        desk: "g10-rates",
        counterparty: "X",
        request_kind: 1,
        headline: "Manual pricing needed",
        alert_worthy: true,
        reason: Number(ord),
      });
      expect(ntf.reason).toBe(member);
    }
  });

  it("omits reason for a null / zero (unspecified) ordinal", () => {
    for (const bad of [null, 0] as const) {
      const ntf = notificationFromWire({
        type: "notification",
        notification_id: "ntf-z",
        kind: 7,
        at_nanos: 1n,
        desk: "g10-rates",
        counterparty: "X",
        request_kind: 1,
        headline: "Manual pricing needed",
        alert_worthy: true,
        reason: bad as unknown as number,
      });
      expect(ntf.reason).toBeUndefined();
    }
  });
});
