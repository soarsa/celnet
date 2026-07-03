// XVA slice — the `price_xva` add-in path (CELNET.XVA): shape a netting set of FX
// vanillas + the single-factor exposure model + the counterparty/own survival curves
// into the typed contract, encode to the wire EXACTLY as the server decodes it
// (`crates/celnet-server/src/ws/codec.rs` `price_xva_request_from_json`, oracle
// `xva_price_request_decodes_prices_and_response_round_trips`), decode a
// `price_xva_response`, and lay out the four scalar adjustments as a spill. The
// add-in holds no XVA math: it is a thin client of the live `celnet-xva` engine over
// the one unversioned contract. Byte-matched to the GUI's `priceXvaRequestToWire` /
// `xvaResultFromWire` (`gui/src/data/wsCodec.ts`).
import { describe, expect, it } from "vitest";
import { Connection, type StreamEvent } from "../src/transport/connection";
import type { WebSocketLike } from "../src/transport/socket";
import type { XvaPricingRequest } from "../src/contract/contract";
import { priceXvaRequestToWire, xvaResultFromWire } from "../src/contract/wsCodec";
import * as e from "../src/contract/enums";
import {
  ShapingError,
  formatXvaSpill,
  shapeXvaRequest,
  shapeXvaSurvivalCurve,
  shapeXvaTrades,
} from "../src/functions/shaping";

// --- in-memory socket (mirrors test/ratesRisk.test.ts) ----------------------
class FakeSocket implements WebSocketLike {
  readyState = 0;
  onopen: (() => void) | null = null;
  onclose: (() => void) | null = null;
  onerror: (() => void) | null = null;
  onmessage: ((data: string) => void) | null = null;
  readonly sent: Record<string, unknown>[] = [];

  send(data: string): void {
    this.sent.push(JSON.parse(data) as Record<string, unknown>);
  }
  close(): void {
    this.readyState = 3;
    this.onclose?.();
  }
  open(): void {
    this.readyState = 1;
    this.onopen?.();
  }
  deliver(frame: Record<string, unknown>): void {
    this.onmessage?.(JSON.stringify(frame));
  }
  sentOfType(type: string): Record<string, unknown>[] {
    return this.sent.filter((f) => f["type"] === type);
  }
}

function makeConn(): { conn: Connection; sock: FakeSocket } {
  const sock = new FakeSocket();
  let seq = 1;
  const conn = new Connection({
    url: "ws://test",
    factory: () => sock,
    stalenessWindowMs: 1000,
    clock: () => 0,
    setTimer: () => seq++,
    clearTimer: () => {},
    requestTimeoutMs: 5000,
  });
  const events: StreamEvent[] = [];
  conn.onEvent((e) => events.push(e));
  return { conn, sock };
}

// A two-trade netting set (a long call + a short put, so the netting is non-trivial),
// a flat counterparty hazard + a piecewise own hazard, and the exposure model.
const REQUEST: XvaPricingRequest = {
  trades: [
    { optionType: "CALL", strike: 1.1, expiryYears: 1, vol: 0.1, notional: 1e7 },
    { optionType: "PUT", strike: 1.2, expiryYears: 2, vol: 0.12, notional: -5e6 },
  ],
  rDom: 0.03,
  rFor: 0.01,
  spot0: 1.15,
  sigma: 0.11,
  paths: 4096,
  seed: 1,
  exposureSteps: 16,
  counterparty: { pillarTimes: [], hazardRates: [0.02] },
  own: {
    pillarTimes: [1, 3],
    hazardRates: [0.01, 0.015],
  },
  lgdCounterparty: 0.6,
  lgdOwn: 0.55,
  fundingSpread: 0.008,
};

describe("XVA wire codec — exact field names the engine decodes", () => {
  it("encodes the request with numeric option types + snake_case scalars + nested curves", () => {
    const wire = priceXvaRequestToWire(REQUEST);
    // trades: numeric-enum option_type + snake_case payoff fields (server `xva_trade_from_json`).
    expect(wire["trades"]).toEqual([
      {
        option_type: e.optionType.toWire("CALL"),
        strike: 1.1,
        expiry_years: 1,
        vol: 0.1,
        notional: 1e7,
      },
      {
        option_type: e.optionType.toWire("PUT"),
        strike: 1.2,
        expiry_years: 2,
        vol: 0.12,
        notional: -5e6,
      },
    ]);
    expect(e.optionType.toWire("CALL")).toBe(0);
    expect(e.optionType.toWire("PUT")).toBe(1);
    // exposure model + MC budget.
    expect(wire["r_dom"]).toBe(0.03);
    expect(wire["r_for"]).toBe(0.01);
    expect(wire["spot0"]).toBe(1.15);
    expect(wire["sigma"]).toBe(0.11);
    expect(wire["paths"]).toBe(4096);
    expect(wire["seed"]).toBe(1);
    expect(wire["exposure_steps"]).toBe(16);
    // A FLAT curve rides an EXPLICIT empty pillar_times + a single hazard (the server
    // `nested(...)` decode REQUIRES the key present, and `f64_array` accepts `[]`).
    expect(wire["counterparty"]).toEqual({ pillar_times: [], hazard_rates: [0.02] });
    // A PIECEWISE curve carries equal-length strictly-increasing pillars + hazards.
    expect(wire["own"]).toEqual({ pillar_times: [1, 3], hazard_rates: [0.01, 0.015] });
    // LGDs + funding spread.
    expect(wire["lgd_counterparty"]).toBe(0.6);
    expect(wire["lgd_own"]).toBe(0.55);
    expect(wire["funding_spread"]).toBe(0.008);
    // request_id / correlation_id are NOT sent (server reads them as zero / absent),
    // byte-identical to the GUI's `priceXvaRequestToWire`.
    expect("request_id" in wire).toBe(false);
    expect("correlation_id" in wire).toBe(false);
  });

  it("decodes a price_xva_response's nested result into the four adjustments", () => {
    const decoded = xvaResultFromWire({
      type: "price_xva_response",
      correlation_id: 42,
      result: {
        cva: 12345.6,
        dva: 2345.6,
        fva: 789.0,
        total_adjustment: 12345.6 - 2345.6 + 789.0,
      },
    });
    expect(decoded).toEqual({
      cva: 12345.6,
      dva: 2345.6,
      fva: 789.0,
      totalAdjustment: 12345.6 - 2345.6 + 789.0,
    });
  });

  it("round-trips a request shaped from cells back through the encoder", () => {
    const shaped = shapeXvaRequest({
      trades: [
        ["C", 1.1, 1, 0.1, 1e7],
        ["P", 1.2, 2, 0.12, -5e6],
      ],
      spot0: 1.15,
      sigma: 0.11,
      rDom: 0.03,
      rFor: 0.01,
      counterparty: [[0.02]],
      own: [
        [1, 0.01],
        [3, 0.015],
      ],
      lgdCounterparty: 0.6,
      lgdOwn: 0.55,
      fundingSpread: 0.008,
    });
    expect(priceXvaRequestToWire(shaped)).toEqual(priceXvaRequestToWire(REQUEST));
  });
});

describe("XVA input shaping — netting set + survival curves", () => {
  it("shapes a trade-per-row netting set, parsing C/P and ignoring blank rows", () => {
    const trades = shapeXvaTrades([
      ["C", 1.1, 1, 0.1, 1e7],
      ["", "", "", "", ""],
      ["PUT", 1.2, 2, 0.12, -5e6],
    ]);
    expect(trades).toEqual([
      { optionType: "CALL", strike: 1.1, expiryYears: 1, vol: 0.1, notional: 1e7 },
      { optionType: "PUT", strike: 1.2, expiryYears: 2, vol: 0.12, notional: -5e6 },
    ]);
  });

  it("rejects a short trade row, a non-positive strike/expiry/vol, and an empty set", () => {
    expect(() => shapeXvaTrades([["C", 1.1, 1, 0.1]])).toThrow(
      /needs \[callPut, strike, expiryYears, vol, notional\]/,
    );
    expect(() => shapeXvaTrades([["C", -1.1, 1, 0.1, 1e7]])).toThrow(/strike must be > 0/);
    expect(() => shapeXvaTrades([["C", 1.1, 0, 0.1, 1e7]])).toThrow(/expiryYears must be > 0/);
    expect(() => shapeXvaTrades([["C", 1.1, 1, 0, 1e7]])).toThrow(/vol must be > 0/);
    expect(() => shapeXvaTrades([["", "", "", "", ""]])).toThrow(/at least one trade/);
    expect(() => shapeXvaTrades([["X", 1.1, 1, 0.1, 1e7]])).toThrow(ShapingError);
  });

  it("reads a FLAT survival curve from a single scalar cell (no pillars)", () => {
    expect(shapeXvaSurvivalCurve([[0.02]], "counterparty")).toEqual({
      pillarTimes: [],
      hazardRates: [0.02],
    });
  });

  it("reads a PIECEWISE survival curve from a [pillarYears, hazard] range", () => {
    expect(
      shapeXvaSurvivalCurve(
        [
          [1, 0.01],
          [3, 0.015],
          ["", ""],
        ],
        "own",
      ),
    ).toEqual({ pillarTimes: [1, 3], hazardRates: [0.01, 0.015] });
  });

  it("rejects non-increasing pillars, a negative hazard, and an empty curve", () => {
    expect(() =>
      shapeXvaSurvivalCurve(
        [
          [3, 0.01],
          [1, 0.015],
        ],
        "own",
      ),
    ).toThrow(/strictly increasing/);
    expect(() => shapeXvaSurvivalCurve([[-0.5]], "counterparty")).toThrow(/hazard must be >= 0/);
    expect(() => shapeXvaSurvivalCurve([["", ""]], "own")).toThrow(/survival curve is empty/);
  });

  it("assembles the full request, defaulting the optional MC controls to the GUI budget", () => {
    const shaped = shapeXvaRequest({
      trades: [["C", 1.1, 1, 0.1, 1e7]],
      spot0: 1.15,
      sigma: 0.11,
      rDom: 0.03,
      rFor: 0.01,
      counterparty: [[0.02]],
      own: [[0.01]],
      lgdCounterparty: 0.6,
      lgdOwn: 0.55,
      fundingSpread: 0.008,
    });
    expect(shaped.paths).toBe(4096);
    expect(shaped.seed).toBe(1);
    expect(shaped.exposureSteps).toBe(16);
  });

  it("rejects a bad spot0/sigma, an out-of-range LGD, and a bad MC count", () => {
    const base = {
      trades: [["C", 1.1, 1, 0.1, 1e7]] as (string | number | boolean)[][],
      spot0: 1.15,
      sigma: 0.11,
      rDom: 0.03,
      rFor: 0.01,
      counterparty: [[0.02]] as (string | number | boolean)[][],
      own: [[0.01]] as (string | number | boolean)[][],
      lgdCounterparty: 0.6,
      lgdOwn: 0.55,
      fundingSpread: 0.008,
    };
    expect(() => shapeXvaRequest({ ...base, spot0: 0 })).toThrow(/spot0 must be > 0/);
    expect(() => shapeXvaRequest({ ...base, sigma: -0.1 })).toThrow(/sigma must be a decimal >= 0/);
    expect(() => shapeXvaRequest({ ...base, lgdCounterparty: 1.5 })).toThrow(/\[0, 1\]/);
    expect(() => shapeXvaRequest({ ...base, paths: 0 })).toThrow(/paths must be a whole number >= 1/);
    expect(() => shapeXvaRequest({ ...base, exposureSteps: 2.5 })).toThrow(
      /exposureSteps must be a whole number >= 1/,
    );
  });
});

describe("XVA spill layout", () => {
  it("lays out the four adjustments + a provenance footer, rectangular", () => {
    const spill = formatXvaSpill(
      { cva: 12345.6, dva: 2345.6, fva: 789.0, totalAdjustment: 10789.0 },
      2,
    );
    expect(spill[0]).toEqual(["cva", 12345.6]);
    expect(spill[1]).toEqual(["dva", 2345.6]);
    expect(spill[2]).toEqual(["fva", 789.0]);
    expect(spill[3]).toEqual(["total_adjustment", 10789.0]);
    expect(String(spill[4]![0])).toMatch(/2 trades netted \| server-priced \(celnet-xva\)/);
    // Office.js custom functions require a rectangular 2-D return.
    const width = spill[0]!.length;
    for (const row of spill) expect(row.length).toBe(width);
  });

  it("singularises the footer for a one-trade set", () => {
    const spill = formatXvaSpill({ cva: 1, dva: 0, fva: 0, totalAdjustment: 1 }, 1);
    expect(String(spill[4]![0])).toMatch(/1 trade netted \|/);
  });
});

describe("Connection.priceXva round-trip", () => {
  it("sends a `price_xva` frame with the wire body (no session token) and decodes the reply", async () => {
    const { conn, sock } = makeConn();
    sock.open();
    const p = conn.priceXva(REQUEST);
    const sent = sock.sentOfType("price_xva")[0]!;
    expect(sent["trades"]).toEqual(priceXvaRequestToWire(REQUEST)["trades"]);
    expect(sent["counterparty"]).toEqual({ pillar_times: [], hazard_rates: [0.02] });
    expect(sent["own"]).toEqual({ pillar_times: [1, 3], hazard_rates: [0.01, 0.015] });
    // `price_xva` is anonymous (a pure calculation) — no session_token rides the frame.
    expect("session_token" in sent).toBe(false);
    const corr = sent["correlation_id"] as number;
    sock.deliver({
      type: "price_xva_response",
      correlation_id: corr,
      result: { cva: 999.5, dva: 100.5, fva: 42.0, total_adjustment: 941.0 },
    });
    const result = await p;
    expect(result).toEqual({ cva: 999.5, dva: 100.5, fva: 42.0, totalAdjustment: 941.0 });
  });

  it("rejects on a typed error frame correlated to the request", async () => {
    const { conn, sock } = makeConn();
    sock.open();
    const p = conn.priceXva(REQUEST);
    const corr = sock.sentOfType("price_xva")[0]!["correlation_id"] as number;
    sock.deliver({ type: "error", correlation_id: corr, message: "invalid_argument: empty netting set" });
    await expect(p).rejects.toThrow(/invalid_argument/);
  });
});
