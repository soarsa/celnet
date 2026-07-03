import { describe, expect, it } from "vitest";

import type {
  RatesCurveSet,
  RatesInstrument,
  RatesQuote,
} from "../src/data/contract";
import { createMockTransport } from "../src/data/mockSource";
import {
  BOND_RFQ_HALF_SPREAD,
  DEFAULT_USD_SOFR_CURVE,
  RATES_RFQ_HALF_SPREAD,
  priceRatesInstrumentOffline,
  ratesRfqTwoWayOffline,
} from "../src/data/ratesPricing";
import {
  ratesCurveSetToWire,
  ratesInstrumentUnionToWire,
  ratesQuoteFromWire,
  ratesQuoteRequestToWire,
  type WireObject,
} from "../src/data/wsCodec";

/**
 * Fixed-income RFQ (`QuoteService.RequestRatesQuote`) client parity: the GUI
 * consumes the `rates-rfq-ws` wire landed in celnet-server (commit ee65c0a). These
 * tests pin (1) the request codec byte-shape the server decoder
 * (`generated_codec::decode_rates_quote_request`) reads, (2) the `rates_quote`
 * reply decode (the server `encode_rates_quote` mirror), and (3) the deterministic
 * offline two-way — its mid equal to the offline `priceRates` fair level EXACTLY,
 * the FI analogue of the server's `quote_rates_two_way` (one FI pricing path). There
 * is NO multi-dealer rates path on the contract (the panel wire `QuoteRequest` is
 * FX-`Instrument` only), so only the single two-way is exercised.
 */

// A small explicit USD-SOFR ladder so the request wire can be pinned byte-for-byte
// against the server's `rates_rfq_ws.rs` `curve_json()` / `ois_json()` shapes.
const SMALL_CURVE: RatesCurveSet = {
  currency: "USD",
  referenceDate: { year: 2026, month: 6, day: 25 },
  pillars: [
    { tenor: { kind: "years", years: 1 }, parRate: 0.042 },
    { tenor: { kind: "years", years: 2 }, parRate: 0.041 },
    { tenor: { kind: "years", years: 5 }, parRate: 0.0405 },
    { tenor: { kind: "years", years: 10 }, parRate: 0.0415 },
  ],
};

const NOTIONAL = 100_000_000;

function oisInstrument(): RatesInstrument {
  return {
    kind: "ois",
    ois: { tenorYears: 5, fixedRate: 0.04, notional: NOTIONAL, direction: "PAY_FIXED" },
  };
}

function bondInstrument(): RatesInstrument {
  return {
    kind: "bond",
    bond: {
      couponRate: 0.05,
      couponFrequency: "SEMI_ANNUAL",
      dayCount: "THIRTY_360_BOND_BASIS",
      maturityDate: { year: 2031, month: 6, day: 25 },
      redemption: 100,
      position: "LONG",
    },
  };
}

describe("rates RFQ request codec (RatesQuoteRequest → wire)", () => {
  it("emits the exact snake_case envelope the server decoder reads", () => {
    const wire = ratesQuoteRequestToWire("fi-rfq-decode", SMALL_CURVE, oisInstrument(), NOTIONAL, "TWO_WAY");
    // Byte-shape pinned against crates/celnet-server/tests/rates_rfq_ws.rs.
    expect(wire).toEqual({
      idempotency_key: "fi-rfq-decode",
      curve_set: {
        currency: "USD",
        reference_date: { year: 2026, month: 6, day: 25 },
        ois_pillars: [
          { tenor: { years: 1 }, par_rate: 0.042 },
          { tenor: { years: 2 }, par_rate: 0.041 },
          { tenor: { years: 5 }, par_rate: 0.0405 },
          { tenor: { years: 10 }, par_rate: 0.0415 },
        ],
      },
      instrument: { ois: { tenor_years: 5, fixed_rate: 0.04, notional: NOTIONAL, side: 0 } },
      notional: NOTIONAL,
      side: 2,
    });
  });

  it("reuses the shared curve/instrument encoders (no duplicate encoding)", () => {
    const inst = oisInstrument();
    const wire = ratesQuoteRequestToWire("k", SMALL_CURVE, inst, NOTIONAL, "SELL");
    expect(wire.curve_set).toEqual(ratesCurveSetToWire(SMALL_CURVE));
    expect(wire.instrument).toEqual(ratesInstrumentUnionToWire(inst));
    expect(wire.side).toBe(1); // SELL
  });

  it("encodes each RFQ side to its wire Side integer (BUY=0, SELL=1, TWO_WAY=2)", () => {
    expect(ratesQuoteRequestToWire("k", SMALL_CURVE, oisInstrument(), NOTIONAL, "BUY").side).toBe(0);
    expect(ratesQuoteRequestToWire("k", SMALL_CURVE, oisInstrument(), NOTIONAL, "SELL").side).toBe(1);
    expect(ratesQuoteRequestToWire("k", SMALL_CURVE, oisInstrument(), NOTIONAL, "TWO_WAY").side).toBe(2);
  });
});

describe("rates RFQ reply codec (rates_quote frame → RatesQuote)", () => {
  const frame: WireObject = {
    type: "rates_quote",
    quote_id: 42,
    idempotency_key: "fi-rfq-decode",
    price: { bid: 0.03995, offer: 0.04005 },
    result: { pv: -12345.6, par_rate: 0.04, pv01: 4200.0, dv01: 4180.0, key_rate_ladder: [10, 20, 30, 40] },
    notional: NOTIONAL,
    epoch_nanos: 1_700_000_000_000_000_000,
    valid_until_nanos: 1_700_000_008_000_000_000,
    correlation_id: 77,
  };

  it("decodes the two-way, risk, size, timestamps and correlation id", () => {
    const q: RatesQuote = ratesQuoteFromWire(frame);
    expect(q.quoteId).toBe(42n);
    expect(q.idempotencyKey).toBe("fi-rfq-decode");
    expect(q.price).toEqual({ bid: 0.03995, offer: 0.04005 });
    expect(q.result.parRate).toBe(0.04);
    expect(q.result.dv01).toBe(4180.0);
    expect(q.result.keyRateLadder).toEqual([10, 20, 30, 40]);
    expect(q.notional).toBe(NOTIONAL);
    expect(q.epochNanos).toBe(1_700_000_000_000_000_000n);
    expect(q.validUntilNanos).toBe(1_700_000_008_000_000_000n);
    expect(q.correlationId).toBe(77n);
  });

  it("treats an absent/null correlation id as undefined (honest absence)", () => {
    const rest: WireObject = { ...frame };
    delete rest.correlation_id;
    expect(ratesQuoteFromWire(rest).correlationId).toBeUndefined();
    expect(ratesQuoteFromWire({ ...rest, correlation_id: null }).correlationId).toBeUndefined();
  });
});

describe("offline rates RFQ two-way (mock transport parity)", () => {
  it("mints an OIS two-way whose mid equals the offline priceRates par rate", async () => {
    const t = createMockTransport();
    const q = await t.requestRatesQuote(DEFAULT_USD_SOFR_CURVE, oisInstrument(), NOTIONAL, "TWO_WAY", "k1");

    expect(q.price.bid).toBeLessThan(q.price.offer);
    const mid = 0.5 * (q.price.bid + q.price.offer);
    // The par rate is side-independent — the landed offline price_rates mid.
    const reference = priceRatesInstrumentOffline(DEFAULT_USD_SOFR_CURVE, oisInstrument()).parRate;
    expect(Math.abs(mid - reference)).toBeLessThanOrEqual(1e-12);
    // The rate market is struck a fixed half-spread either side (1bp wide).
    expect(q.price.offer - q.price.bid).toBeCloseTo(2 * RATES_RFQ_HALF_SPREAD, 15);

    expect(q.notional).toBe(NOTIONAL);
    expect(q.validUntilNanos).toBeGreaterThan(q.epochNanos);
    expect(q.result.keyRateLadder.length).toBe(DEFAULT_USD_SOFR_CURVE.pillars.length);
    expect(q.idempotencyKey).toBe("k1");
  });

  it("mints a cash-bond two-way as a clean-PRICE market (10-cent wide)", async () => {
    const t = createMockTransport();
    const q = await t.requestRatesQuote(DEFAULT_USD_SOFR_CURVE, bondInstrument(), 25_000_000, "TWO_WAY", "k2");
    expect(q.price.bid).toBeLessThan(q.price.offer);
    // A cash bond quotes a clean price per 100 face, not a rate — a wider market.
    expect(q.price.offer - q.price.bid).toBeCloseTo(2 * BOND_RFQ_HALF_SPREAD, 12);
    const mid = 0.5 * (q.price.bid + q.price.offer);
    expect(mid).toBeGreaterThan(0);
    expect(q.notional).toBe(25_000_000);
  });

  it("wires the transport reply straight through the offline two-way helper", async () => {
    const t = createMockTransport();
    const q = await t.requestRatesQuote(DEFAULT_USD_SOFR_CURVE, oisInstrument(), NOTIONAL, "BUY", "k3");
    const direct = ratesRfqTwoWayOffline(DEFAULT_USD_SOFR_CURVE, oisInstrument(), "BUY");
    expect(q.price.bid).toBe(direct.bid);
    expect(q.price.offer).toBe(direct.offer);
    expect(q.result).toEqual(direct.result);
  });

  it("signs the risk from the RFQ envelope side, not the instrument arm (TWO_WAY ⇒ receive-fixed magnitude)", () => {
    // A pay-fixed OIS arm with a TWO_WAY envelope reports the canonical receive-fixed
    // (SELL) sign — matching the server resolve_pricing_side; the mid is unchanged.
    const twoWay = ratesRfqTwoWayOffline(DEFAULT_USD_SOFR_CURVE, oisInstrument(), "TWO_WAY");
    const sell = ratesRfqTwoWayOffline(DEFAULT_USD_SOFR_CURVE, oisInstrument(), "SELL");
    expect(twoWay.result).toEqual(sell.result);
    // The BUY (pay-fixed) side has the opposite PV sign to SELL.
    const buy = ratesRfqTwoWayOffline(DEFAULT_USD_SOFR_CURVE, oisInstrument(), "BUY");
    expect(Math.sign(buy.result.pv)).toBe(-Math.sign(sell.result.pv));
  });
});
