/**
 * Street-side EXECUTION analytics — the codec contract.
 *
 * The single property under test throughout is the one the feature exists to
 * protect: a datum the server did NOT observe arrives as `null` and must decode to
 * `undefined`, so the UI renders an explicit absence marker. A silently-substituted
 * `0` would be a fabricated metric, which is exactly the failure this whole surface
 * was built to eliminate.
 */
import { describe, expect, it } from "vitest";

import {
  listStreetOrdersRequestToWire,
  listStreetOrdersResponseFromWire,
  streetBreakdownRowFromWire,
  streetOrderFromWire,
} from "../src/data/wsCodec";
import { MockTransport } from "../src/data/mockSource";
import type { WireObject } from "../src/data/wsCodec";

/** A fully-populated named-LP fill as the server encodes it. */
function filledWire(): WireObject {
  return {
    order_id: "SO-1",
    ts_nanos: 1_700_000_000_000_000_000,
    lp_id: "citigroup-sim",
    venue: "named_lp",
    instrument: "USSW10",
    family: "ois",
    tenor_years: 10,
    side: "sell",
    requested_qty: 4000,
    filled_qty: 4000,
    requested_price: 0.03,
    filled_price: 0.03005,
    slippage_bp: 0.5,
    outcome: "filled",
    reason: null,
    competitors: [
      { lp_id: "citigroup-sim", price: 0.03005 },
      { lp_id: "jpm-sim", price: 0.03007 },
    ],
    parent_hedge_id: "HDG-1",
    parent_position_id: 42,
    order_type: null,
    time_in_force: null,
    response_latency_nanos: null,
  };
}

describe("listStreetOrdersRequestToWire", () => {
  it("omits an absent window and every blank filter", () => {
    expect(listStreetOrdersRequestToWire()).toEqual({});
    // A blank control must NOT become a filter that matches nothing.
    expect(
      listStreetOrdersRequestToWire(undefined, { lpId: "", family: "", instrument: "" }),
    ).toEqual({});
  });

  it("frames the window, the filters and the breakdown dimension", () => {
    const m = listStreetOrdersRequestToWire(
      { fromNanos: 100n, toNanos: 900n },
      {
        lpId: "jpm-sim",
        family: "bond_future",
        outcome: "rejected",
        parentHedgeId: "HDG-7",
        dimension: "tenor_bucket",
        limit: 250,
      },
    );
    expect(m).toEqual({
      from_nanos: 100,
      to_nanos: 900,
      lp_id: "jpm-sim",
      family: "bond_future",
      outcome: "rejected",
      parent_hedge_id: "HDG-7",
      dimension: "tenor_bucket",
      limit: 250,
    });
  });
});

describe("streetOrderFromWire", () => {
  it("decodes a named-LP fill with its full economics and competing panel", () => {
    const o = streetOrderFromWire(filledWire());
    expect(o.orderId).toBe("SO-1");
    expect(o.lpId).toBe("citigroup-sim");
    expect(o.venue).toBe("named_lp");
    expect(o.family).toBe("ois");
    expect(o.side).toBe("sell");
    expect(o.filledPrice).toBe(0.03005);
    expect(o.slippageBp).toBe(0.5);
    expect(o.outcome).toBe("filled");
    expect(o.competitors).toHaveLength(2);
    expect(o.competitors[1]).toEqual({ lpId: "jpm-sim", price: 0.03007 });
    expect(o.parentHedgeId).toBe("HDG-1");
    expect(o.parentPositionId).toBe(42n);
  });

  it("decodes every UNOBSERVED datum to undefined, never to zero", () => {
    const o = streetOrderFromWire(filledWire());
    // The routing seam lifts a standing firm price in-process: it issues no typed
    // order and has no round trip to time, so all three are genuinely absent.
    expect(o.orderType).toBeUndefined();
    expect(o.timeInForce).toBeUndefined();
    expect(o.responseLatencyNanos).toBeUndefined();
    // The distinction that matters: absent is NOT zero.
    expect(o.responseLatencyNanos).not.toBe(0);
  });

  it("carries the venue's own reason code verbatim", () => {
    // The qualifier on a routed order is the counterparty's Text(58) code — it is
    // passed through untranslated so the blotter reports what the venue said.
    const o = streetOrderFromWire({ ...filledWire(), reason: "IOC_DEPTH_EXHAUSTED" });
    expect(o.reason).toBe("IOC_DEPTH_EXHAUSTED");
  });

  it("carries an unroutable member's configuration reason, distinct from a refusal", () => {
    // `no_order_endpoint` is OUR missing setup, not a counterparty saying no — the
    // blotter must be able to tell those apart.
    const o = streetOrderFromWire({ ...filledWire(), reason: "no_order_endpoint" });
    expect(o.reason).toBe("no_order_endpoint");
  });

  it("decodes a composite backstop with NO lp and no fill price", () => {
    const o = streetOrderFromWire({
      ...filledWire(),
      lp_id: null,
      venue: "composite_backstop",
      outcome: "no_liquidity",
      reason: "no_firm_lp_price",
      filled_qty: 0,
      filled_price: null,
      slippage_bp: null,
      competitors: [],
    });
    expect(o.lpId).toBeUndefined();
    expect(o.venue).toBe("composite_backstop");
    expect(o.reason).toBe("no_firm_lp_price");
    expect(o.filledPrice).toBeUndefined();
    expect(o.slippageBp).toBeUndefined();
    expect(o.competitors).toEqual([]);
  });

  it("THROWS on an unknown venue or outcome token rather than guessing", () => {
    // A silent fallback here is how a contract drift renders as plausible-but-wrong
    // data. One contract, one vocabulary — fail loudly.
    expect(() => streetOrderFromWire({ ...filledWire(), venue: "dark_pool" })).toThrow(
      /unknown venue token/,
    );
    expect(() => streetOrderFromWire({ ...filledWire(), outcome: "maybe" })).toThrow(
      /unknown outcome token/,
    );
  });
});

describe("streetBreakdownRowFromWire", () => {
  const base: WireObject = {
    dimension: "lp",
    key: "citigroup-sim",
    orders: 4,
    filled: 3,
    partially_filled: 1,
    rejected: 0,
    cancelled: 0,
    expired: 0,
    last_look_pulled: 0,
    no_liquidity: 0,
    composite_backstop: 0,
    requested_qty: 40_000,
    filled_qty: 32_500,
    fill_ratio: 0.8125,
    win_rate: 1,
    mean_slippage_bp: 0.4,
    mean_response_latency_nanos: null,
    last_look_rate: 0,
    mean_cover: null,
    mean_competitors: 2.5,
  };

  it("keeps a real zero as a zero and an absent ratio as undefined", () => {
    const r = streetBreakdownRowFromWire(base);
    // `last_look_rate: 0` is an OBSERVED zero — 0 pulls out of 4 orders.
    expect(r.lastLookRate).toBe(0);
    expect(r.rejected).toBe(0);
    // `mean_cover: null` means no order in the group HAD a cover — absent.
    expect(r.meanCover).toBeUndefined();
    expect(r.meanResponseLatencyNanos).toBeUndefined();
    expect(r.fillRatio).toBeCloseTo(0.8125, 12);
    expect(r.meanCompetitors).toBe(2.5);
  });

  it("preserves the explicit COMPOSITE and unattributed buckets as distinct keys", () => {
    expect(streetBreakdownRowFromWire({ ...base, key: "COMPOSITE" }).key).toBe("COMPOSITE");
    expect(streetBreakdownRowFromWire({ ...base, key: "" }).key).toBe("");
  });

  it("keeps a venue REFUSAL and a venue CANCEL as distinct outcomes", () => {
    // "would not" (OrdStatus=8) and "could not" (OrdStatus=4) are different facts
    // about an LP; flattening them would make a deep, willing counterparty look
    // like a refusing one.
    const r = streetBreakdownRowFromWire({ ...base, rejected: 2, cancelled: 3 });
    expect(r.rejected).toBe(2);
    expect(r.cancelled).toBe(3);
    expect(streetOrderFromWire({ ...filledWire(), outcome: "cancelled" }).outcome).toBe(
      "cancelled",
    );
  });

  it("THROWS on an unknown grouping dimension", () => {
    expect(() => streetBreakdownRowFromWire({ ...base, dimension: "desk" })).toThrow(
      /unknown dimension token/,
    );
  });
});

describe("listStreetOrdersResponseFromWire", () => {
  it("carries the pre-page total so the UI never implies it has every row", () => {
    const v = listStreetOrdersResponseFromWire({
      orders: [filledWire()],
      breakdown: [],
      total_matching: 5312,
    });
    expect(v.orders).toHaveLength(1);
    expect(v.totalMatching).toBe(5312);
  });

  it("decodes a genuinely empty reply as empty, not as absent data", () => {
    const v = listStreetOrdersResponseFromWire({
      orders: [],
      breakdown: [],
      total_matching: 0,
    });
    expect(v).toEqual({ orders: [], breakdown: [], totalMatching: 0 });
  });
});

describe("the offline mock", () => {
  it("returns an HONEST empty street-order view rather than inventing street activity", async () => {
    const v = await new MockTransport().listStreetOrders();
    expect(v).toEqual({ orders: [], breakdown: [], totalMatching: 0 });
  });
});
