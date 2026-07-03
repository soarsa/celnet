// Linear-rates portfolio risk slice — the `aggregate_rates_risk` add-in path
// (CELNET.RATESRISK): shape an OIS book + optional scope into the typed contract,
// encode to the wire EXACTLY as the server decodes it
// (`crates/celnet-server/src/ws/codec.rs` `aggregate_rates_risk_*`, oracle
// `aggregate_rates_risk_request_decodes`), decode an `aggregate_rates_risk_response`,
// and lay out the netted per-currency node tree as a spill. The add-in holds no
// rates-risk math: it is a thin client of the live `celnet-rates` engine + the
// server-owned additive roll-up over the one unversioned contract.
import { describe, expect, it } from "vitest";
import { Connection, type StreamEvent } from "../src/transport/connection";
import type { WebSocketLike } from "../src/transport/socket";
import type { RatesCurveSet } from "../src/contract/contract";
import {
  aggregateRatesRiskRequest,
  aggregateRatesRiskResponseFromWire,
  type AggregateRatesRiskRequest,
  type RatesPosition,
} from "../src/contract/riskCodec";
import { ratesCurveSetToWire, ratesInstrumentToWire } from "../src/contract/wsCodec";
import {
  ShapingError,
  formatRatesRiskSpill,
  parseRatesRiskScope,
  shapeRatesRiskPositions,
} from "../src/functions/shaping";

// --- in-memory socket (mirrors test/rates.test.ts) --------------------------
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

// A 3-pillar self-discounting USD-SOFR curve + a two-position book (a payer and a
// receiver, so the netting is non-trivial).
const CURVE: RatesCurveSet = {
  currency: "USD",
  referenceDate: { year: 2026, month: 6, day: 25 },
  pillars: [
    { tenorYears: 2, parRate: 0.0405 },
    { tenorYears: 5, parRate: 0.041 },
    { tenorYears: 10, parRate: 0.0418 },
  ],
};

const POSITIONS: readonly RatesPosition[] = [
  {
    positionId: 1n,
    entity: 1,
    book: 100,
    instrument: { tenorYears: 5, fixedRate: 0.0405, notional: 1e8, direction: "PAY_FIXED" },
  },
  {
    positionId: 2n,
    entity: 1,
    book: 100,
    instrument: { tenorYears: 10, fixedRate: 0.0418, notional: 5e7, direction: "RECEIVE_FIXED" },
  },
];

describe("rates-risk wire codec — exact field names the engine decodes", () => {
  it("encodes the request with a NESTED-tenor curve_set, positions, and a grant-all principal", () => {
    const wire = aggregateRatesRiskRequest({ curveSet: CURVE, positions: POSITIONS });
    // curve_set reuses the shared price_rates encoder (nested `tenor:{years}`).
    expect(wire["curve_set"]).toEqual(ratesCurveSetToWire(CURVE));
    expect((wire["curve_set"] as Record<string, unknown>)["ois_pillars"]).toEqual([
      { tenor: { years: 2 }, par_rate: 0.0405 },
      { tenor: { years: 5 }, par_rate: 0.041 },
      { tenor: { years: 10 }, par_rate: 0.0418 },
    ]);
    // positions: { position_id, entity, book, instrument:{ ois:{...} } }.
    expect(wire["positions"]).toEqual([
      { position_id: 1, entity: 1, book: 100, instrument: ratesInstrumentToWire(POSITIONS[0]!.instrument) },
      { position_id: 2, entity: 1, book: 100, instrument: ratesInstrumentToWire(POSITIONS[1]!.instrument) },
    ]);
    // The payer maps to side 0, the receiver to side 1 (the shared OIS encoder).
    expect(
      ((wire["positions"] as Record<string, unknown>[])[0]!["instrument"] as Record<string, Record<string, unknown>>)[
        "ois"
      ]!["side"],
    ).toBe(0);
    expect(
      ((wire["positions"] as Record<string, unknown>[])[1]!["instrument"] as Record<string, Record<string, unknown>>)[
        "ois"
      ]!["side"],
    ).toBe(1);
    // No principal supplied ⇒ an EXPLICIT grant-all clears the server deny-by-default edge.
    expect(wire["principal"]).toEqual({ grant_all: true, grants: [], denies: [] });
    // No scope ⇒ the key is absent (the whole book).
    expect("scope" in wire).toBe(false);
  });

  it("emits the (entity, book, ccy) scope only for the present fields", () => {
    const wire = aggregateRatesRiskRequest({
      curveSet: CURVE,
      positions: POSITIONS,
      scope: { book: 100, ccy: "USD" },
    });
    expect(wire["scope"]).toEqual({ book: 100, ccy: "USD" });
  });

  it("passes an explicit entitlement principal through verbatim (deny-wins)", () => {
    const wire = aggregateRatesRiskRequest({
      curveSet: CURVE,
      positions: POSITIONS,
      principal: {
        grantAll: false,
        grants: [{ scopes: [{ dimension: "BOOK", value: 100n }] }],
        denies: [],
      },
    });
    expect(wire["principal"]).toEqual({
      grant_all: false,
      grants: [{ scopes: [{ dimension: 2, value: 100 }] }],
      denies: [],
    });
  });

  it("decodes an aggregate_rates_risk_response into per-currency netted nodes + ladder", () => {
    const decoded = aggregateRatesRiskResponseFromWire({
      type: "aggregate_rates_risk_response",
      correlation_id: 42,
      nodes: [
        {
          ccy: "USD",
          net_pv: 123_456.78,
          net_pv01: 4_500.0,
          net_dv01: 4_520.0,
          key_rate_ladder: [
            { tenor_years: 2, dv01: 900.0 },
            { tenor_years: 5, dv01: 2_100.0 },
            { tenor_years: 10, dv01: 1_520.0 },
          ],
        },
      ],
    });
    expect(decoded.nodes).toHaveLength(1);
    expect(decoded.nodes[0]).toEqual({
      ccy: "USD",
      netPv: 123_456.78,
      netPv01: 4_500.0,
      netDv01: 4_520.0,
      keyRateLadder: [
        { tenorYears: 2, dv01: 900.0 },
        { tenorYears: 5, dv01: 2_100.0 },
        { tenorYears: 10, dv01: 1_520.0 },
      ],
    });
  });

  it("decodes an empty rollup (no nodes) as an empty node list", () => {
    expect(aggregateRatesRiskResponseFromWire({ nodes: [] }).nodes).toEqual([]);
    expect(aggregateRatesRiskResponseFromWire({}).nodes).toEqual([]);
  });
});

describe("rates-risk input shaping — positions + scope", () => {
  it("shapes an OIS-per-row book, assigning 1-based position ids and ignoring blank rows", () => {
    const positions = shapeRatesRiskPositions([
      [5, 0.0405, "PAY_FIXED", 1e8, 1, 100],
      ["", "", "", "", "", ""],
      ["10Y", 0.0418, "receiver", 5e7],
    ]);
    expect(positions).toEqual([
      {
        positionId: 1n,
        entity: 1,
        book: 100,
        instrument: { tenorYears: 5, fixedRate: 0.0405, notional: 1e8, direction: "PAY_FIXED" },
      },
      {
        // entity/book default to 0 when the columns are absent.
        positionId: 2n,
        entity: 0,
        book: 0,
        instrument: { tenorYears: 10, fixedRate: 0.0418, notional: 5e7, direction: "RECEIVE_FIXED" },
      },
    ]);
  });

  it("rejects a short row, a bad booking id, and an empty book", () => {
    expect(() => shapeRatesRiskPositions([[5, 0.04, "PAY_FIXED"]])).toThrow(
      /needs \[tenorYears, fixedRate, direction, notional\]/,
    );
    expect(() => shapeRatesRiskPositions([[5, 0.04, "PAY_FIXED", 1e8, -1]])).toThrow(
      /entity must be a non-negative integer/,
    );
    expect(() => shapeRatesRiskPositions([["", "", "", ""]])).toThrow(/at least one OIS position/);
    expect(() => shapeRatesRiskPositions([[5, 0.04, "SELL", 1e8]])).toThrow(ShapingError);
  });

  it("parses the (entity, book, ccy) scope tokens and treats ALL/empty as no filter", () => {
    expect(parseRatesRiskScope(undefined)).toBeUndefined();
    expect(parseRatesRiskScope("")).toBeUndefined();
    expect(parseRatesRiskScope("ALL")).toBeUndefined();
    expect(parseRatesRiskScope("FIRM")).toBeUndefined();
    expect(parseRatesRiskScope("BOOK:100")).toEqual({ book: 100 });
    expect(parseRatesRiskScope("ENTITY:1, book=100 ; CCY:usd")).toEqual({
      entity: 1,
      book: 100,
      ccy: "USD",
    });
  });

  it("rejects a malformed scope token, key, ccy, and uint", () => {
    expect(() => parseRatesRiskScope("DESK:1")).toThrow(/invalid scope key/);
    expect(() => parseRatesRiskScope("BOOK")).toThrow(/invalid scope token/);
    expect(() => parseRatesRiskScope("CCY:US")).toThrow(/invalid scope ccy/);
    expect(() => parseRatesRiskScope("BOOK:-2")).toThrow(/non-negative integer/);
  });
});

describe("rates-risk spill layout", () => {
  it("lays out a header + one row per ccy node with the shared key-rate ladder columns", () => {
    const spill = formatRatesRiskSpill([
      {
        ccy: "USD",
        netPv: 123_456.78,
        netPv01: 4_500,
        netDv01: 4_520,
        keyRateLadder: [
          { tenorYears: 2, dv01: 900 },
          { tenorYears: 5, dv01: 2_100 },
          { tenorYears: 10, dv01: 1_520 },
        ],
      },
    ]);
    expect(spill[0]).toEqual([
      "ccy",
      "net_pv",
      "net_pv01",
      "net_dv01",
      "kr_dv01[2Y]",
      "kr_dv01[5Y]",
      "kr_dv01[10Y]",
    ]);
    expect(spill[1]).toEqual(["USD", 123_456.78, 4_500, 4_520, 900, 2_100, 1_520]);
    expect(String(spill[2]![0])).toMatch(/1 currency \| server-netted/);
    // Office.js custom functions require a rectangular 2-D return.
    const width = spill[0]!.length;
    for (const row of spill) expect(row.length).toBe(width);
  });

  it("unions ladder tenors across ccy nodes and blanks a missing bucket (never a spurious zero)", () => {
    const spill = formatRatesRiskSpill([
      { ccy: "EUR", netPv: 10, netPv01: 1, netDv01: 1, keyRateLadder: [{ tenorYears: 2, dv01: 1 }] },
      { ccy: "USD", netPv: 20, netPv01: 2, netDv01: 2, keyRateLadder: [{ tenorYears: 5, dv01: 2 }] },
    ]);
    // Sorted union of {2, 5}.
    expect(spill[0]).toEqual(["ccy", "net_pv", "net_pv01", "net_dv01", "kr_dv01[2Y]", "kr_dv01[5Y]"]);
    // EUR has the 2Y bucket, blank at 5Y; USD the reverse.
    expect(spill[1]).toEqual(["EUR", 10, 1, 1, 1, ""]);
    expect(spill[2]).toEqual(["USD", 20, 2, 2, "", 2]);
    expect(String(spill[3]![0])).toMatch(/2 currencies \| server-netted/);
  });

  it("spills an honest empty-state when no positions net into scope", () => {
    const spill = formatRatesRiskSpill([]);
    expect(spill).toHaveLength(2);
    expect(String(spill[1]![0])).toMatch(/no rates positions in scope/);
  });
});

describe("Connection.aggregateRatesRisk round-trip", () => {
  it("sends an `aggregate_rates_risk` frame with the wire body and decodes the reply", async () => {
    const { conn, sock } = makeConn();
    sock.open();
    const request: AggregateRatesRiskRequest = { curveSet: CURVE, positions: POSITIONS };
    const body = aggregateRatesRiskRequest(request);
    const p = conn.aggregateRatesRisk(body);
    const sent = sock.sentOfType("aggregate_rates_risk")[0]!;
    expect(sent["curve_set"]).toEqual(ratesCurveSetToWire(CURVE));
    expect(sent["principal"]).toEqual({ grant_all: true, grants: [], denies: [] });
    const corr = sent["correlation_id"] as number;
    sock.deliver({
      type: "aggregate_rates_risk_response",
      correlation_id: corr,
      nodes: [
        {
          ccy: "USD",
          net_pv: 999.5,
          net_pv01: 4_500,
          net_dv01: 4_520,
          key_rate_ladder: [
            { tenor_years: 2, dv01: 900 },
            { tenor_years: 5, dv01: 2_100 },
            { tenor_years: 10, dv01: 1_520 },
          ],
        },
      ],
    });
    const reply = await p;
    const decoded = aggregateRatesRiskResponseFromWire(reply);
    expect(decoded.nodes[0]!.ccy).toBe("USD");
    expect(decoded.nodes[0]!.netPv).toBe(999.5);
    expect(decoded.nodes[0]!.keyRateLadder).toHaveLength(3);
  });

  it("rejects on a typed error frame correlated to the request", async () => {
    const { conn, sock } = makeConn();
    sock.open();
    const p = conn.aggregateRatesRisk(aggregateRatesRiskRequest({ curveSet: CURVE, positions: POSITIONS }));
    const corr = sock.sentOfType("aggregate_rates_risk")[0]!["correlation_id"] as number;
    sock.deliver({ type: "error", correlation_id: corr, message: "permission_denied: rates risk" });
    await expect(p).rejects.toThrow(/permission_denied/);
  });
});
