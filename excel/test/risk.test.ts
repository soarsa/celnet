import { describe, expect, it } from "vitest";
import {
  ShapingError,
  formatLimitsSpill,
  formatPositionsSpill,
  formatRiskSpill,
  parseNumeraireCcy,
  parseRiskDimension,
  parseRiskScope,
  shapeReportingNumeraire,
} from "../src/functions/shaping";
import {
  aggregateRiskRequest,
  aggregateRiskResponseFromWire,
  entitlementPrincipalToWire,
  limitStatusRequest,
  limitStatusResponseFromWire,
  listPositionsRequest,
  listPositionsResponseFromWire,
  riskDimensionFromWire,
  riskDimensionToWire,
  riskScopeToWire,
} from "../src/contract/riskCodec";

describe("risk argument shaping", () => {
  it("parses dimensions including the short aliases (default FIRM)", () => {
    expect(parseRiskDimension(undefined)).toBe("FIRM");
    expect(parseRiskDimension("")).toBe("FIRM");
    expect(parseRiskDimension("desk")).toBe("DESK");
    expect(parseRiskDimension("PAIR")).toBe("CCY_PAIR");
    expect(parseRiskDimension("loc")).toBe("LOCATION");
    expect(parseRiskDimension("LE")).toBe("ENTITY");
    expect(() => parseRiskDimension("nope")).toThrow(ShapingError);
  });

  it("parses a numeraire ccy and rejects a malformed one", () => {
    expect(parseNumeraireCcy("usd")).toBe("USD");
    expect(parseNumeraireCcy(undefined)).toBe("USD");
    expect(() => parseNumeraireCcy("US")).toThrow(ShapingError);
  });

  it("shapes a reporting numeraire from a [ccy, rate] range, dropping the numeraire's own row", () => {
    const reporting = shapeReportingNumeraire("USD", [
      ["EUR", 1.1],
      ["JPY", 0.0067],
      ["USD", 1.0], // implicit 1.0 — dropped
      ["", ""], // blank row — skipped
    ]);
    expect(reporting.numeraire).toBe("USD");
    expect(reporting.rates).toEqual([
      { ccy: "EUR", rate: 1.1 },
      { ccy: "JPY", rate: 0.0067 },
    ]);
  });

  it("rejects a non-positive or non-finite numeraire rate (no silent leg drop)", () => {
    expect(() => shapeReportingNumeraire("USD", [["EUR", 0]])).toThrow(ShapingError);
    expect(() => shapeReportingNumeraire("USD", [["EUR", "x"]])).toThrow(ShapingError);
    expect(() => shapeReportingNumeraire("USD", [["E", 1.1]])).toThrow(ShapingError);
  });

  it("parses a scope DIM:value and treats ALL/FIRM/empty as the whole book", () => {
    expect(parseRiskScope(undefined)).toBeUndefined();
    expect(parseRiskScope("ALL")).toBeUndefined();
    expect(parseRiskScope("FIRM")).toBeUndefined();
    expect(parseRiskScope("DESK:99")).toEqual({ dimension: "DESK", value: 99n });
    expect(parseRiskScope("book=7")).toEqual({ dimension: "BOOK", value: 7n });
    expect(() => parseRiskScope("DESK")).toThrow(ShapingError);
  });
});

describe("risk codec — enum round-trip and request shaping", () => {
  it("round-trips RiskDimension by proto enum number", () => {
    for (const d of ["FIRM", "TRADER", "BOOK", "DESK", "CCY_PAIR", "LOCATION", "ENTITY"] as const) {
      expect(riskDimensionFromWire(riskDimensionToWire(d))).toBe(d);
    }
    // Canonical proto numbers.
    expect(riskDimensionToWire("FIRM")).toBe(0);
    expect(riskDimensionToWire("CCY_PAIR")).toBe(4);
    expect(riskDimensionToWire("ENTITY")).toBe(6);
  });

  it("builds an aggregate_risk body with snake_case keys + numeric enum + defaults", () => {
    const body = aggregateRiskRequest({
      dimension: "DESK",
      numeraire: { numeraire: "USD", rates: [{ ccy: "EUR", rate: 1.1 }] },
      scope: { dimension: "DESK", value: 99n },
    });
    expect(body["dimension"]).toBe(3);
    expect(body["numeraire"]).toEqual({ numeraire: "USD", rates: [{ ccy: "EUR", rate: 1.1 }] });
    expect(body["scope"]).toEqual({ dimension: 3, value: 99 });
    // Defaults present and inert.
    expect(body["vega_pillars"]).toEqual([]);
    expect(body["var_spot_shocks"]).toEqual([]);
    expect(body["var_alpha"]).toBe(0);
    expect(body["curvature_risk_weight"]).toBe(0);
    // No principal sent ⇒ server grant-all default (key omitted).
    expect("principal" in body).toBe(false);
  });

  it("omits scope/principal when absent (grant-all show-all-now)", () => {
    const body = listPositionsRequest({});
    expect("scope" in body).toBe(false);
    expect("principal" in body).toBe(false);
  });

  it("encodes an explicit entitlement principal (deny-wins, scoped)", () => {
    const p = entitlementPrincipalToWire({
      grantAll: false,
      grants: [{ scopes: [{ dimension: "DESK", value: 99n }] }],
      denies: [{ scopes: [{ dimension: "BOOK", value: 7n }] }],
    });
    expect(p).toEqual({
      grant_all: false,
      grants: [{ scopes: [{ dimension: 3, value: 99 }] }],
      denies: [{ scopes: [{ dimension: 2, value: 7 }] }],
    });
  });

  it("builds a limit_status body keyed on a scope", () => {
    const body = limitStatusRequest({
      scope: { dimension: "DESK", value: 99n },
      numeraire: { numeraire: "USD", rates: [] },
      varSpotShocks: [-0.01, 0, 0.01],
      varAlpha: 0.99,
    });
    expect(body["scope"]).toEqual(riskScopeToWire({ dimension: "DESK", value: 99n }));
    expect(body["var_spot_shocks"]).toEqual([-0.01, 0, 0.01]);
    expect(body["var_alpha"]).toBe(0.99);
  });
});

describe("risk codec — response decoding", () => {
  it("decodes an aggregate_risk_response with additive + presence-tracked non-additive", () => {
    const result = aggregateRiskResponseFromWire({
      type: "aggregate_risk_response",
      dimension: 3,
      numeraire: "USD",
      nodes: [
        {
          dimension: 3,
          group: 99,
          additive: {
            delta_numeraire: 1234.5,
            delta_vector: [{ ccy: "EUR", amount: 1000 }, { ccy: "USD", amount: -1100 }],
            gamma: 5.5,
            vega_numeraire: 42.0,
            theta: -3.0,
            vanna: 1,
            volga: 2,
            charm: 3,
            speed: 4,
            zomma: 5,
            color: 6,
            premium_numeraire: 9999.0,
            vega_ladder: [{ pillar: { tenor_days: 365, delta_bp: 5000 }, vega: 42.0 }],
          },
          // VaR present, ES/curvature absent (null) — presence-tracked.
          nonadditive: { var: 250.0, es: null, var_alpha: 0.99, curvature_spot: null },
          position_count: 4,
        },
      ],
    });
    expect(result.dimension).toBe("DESK");
    expect(result.numeraire).toBe("USD");
    expect(result.nodes).toHaveLength(1);
    const node = result.nodes[0]!;
    expect(node.group).toBe(99n);
    expect(node.positionCount).toBe(4);
    expect(node.additive.deltaNumeraire).toBe(1234.5);
    expect(node.additive.deltaVector).toEqual([
      { ccy: "EUR", amount: 1000 },
      { ccy: "USD", amount: -1100 },
    ]);
    expect(node.additive.vegaLadder[0]).toEqual({
      pillar: { tenorDays: 365, deltaBp: 5000 },
      vega: 42.0,
    });
    // Presence-tracked: var present, es/curvature ABSENT (never a spurious zero).
    expect(node.nonadditive.var).toBe(250.0);
    expect(node.nonadditive.es).toBeUndefined();
    expect(node.nonadditive.curvatureSpot).toBeUndefined();
    expect(node.nonadditive.varAlpha).toBe(0.99);
  });

  it("decodes a list_positions_response leaf with the camelCase attribution chain", () => {
    const result = listPositionsResponseFromWire({
      type: "list_positions_response",
      positions: [
        {
          position_id: 42,
          org: {
            trader: 1,
            book: 7,
            desk: 3,
            ccy_pair: { base: "EUR", quote: "USD" },
            location: 1,
            entity: 1,
          },
          option_type: 1, // PUT
          notional_base: -1_000_000,
          inputs: { spot: 1.1, strike: 1.12, vol: 0.105, t: 1.0, r_dom: 0.02, r_for: 0.01 },
          surface_version: 5,
          attribution: {
            heldBy: { book: "EURUSD-DESK", owner: { trader: "jdoe" } },
            quotedBy: { book: "AUTO", owner: { autoPricer: "gk-1" } },
            won: true,
            lpCount: 3,
          },
        },
      ],
    });
    expect(result.positions).toHaveLength(1);
    const p = result.positions[0]!;
    expect(p.positionId).toBe(42n);
    expect(p.optionType).toBe("PUT");
    expect(p.notionalBase).toBe(-1_000_000);
    expect(p.org.book).toBe(7);
    expect(p.org.ccyPair).toEqual({ base: "EUR", quote: "USD" });
    expect(p.surfaceVersion).toBe(5n);
    expect(p.attribution).toBe("held EURUSD-DESK/jdoe | quoted AUTO/auto:gk-1");
  });

  it("decodes a limit_status_response with worst RAG + hard breach", () => {
    const result = limitStatusResponseFromWire({
      type: "limit_status_response",
      scope: { dimension: 3, value: 99 },
      limits: [
        {
          metric: 0, // DELTA
          vega_pillar: { tenor_days: 0, delta_bp: 0 },
          tenor_days: 0,
          cap: 1000,
          exposure: 1500,
          ratio: 1.5,
          status: 3, // BREACH
          enforcement: 1, // HARD
          headroom: -500,
        },
      ],
      worst: 3,
      hard_breach: true,
    });
    expect(result.scope).toEqual({ dimension: "DESK", value: 99n });
    expect(result.limits[0]!.metric).toBe("DELTA");
    expect(result.limits[0]!.status).toBe("BREACH");
    expect(result.limits[0]!.enforcement).toBe("HARD");
    expect(result.worst).toBe("BREACH");
    expect(result.hardBreach).toBe(true);
  });
});

describe("risk spill formatting", () => {
  it("formats CELNET.RISK as a node grid with a numeraire footer; absent non-additive ⇒ blank", () => {
    const spill = formatRiskSpill("FIRM", "USD", [
      {
        dimension: "FIRM",
        group: 0n,
        additive: {
          deltaNumeraire: 1000,
          deltaVector: [],
          gamma: 2,
          vegaNumeraire: 3,
          theta: 4,
          vanna: 5,
          volga: 6,
          charm: 7,
          speed: 8,
          zomma: 9,
          color: 10,
          premiumNumeraire: 5000,
          vegaLadder: [],
        },
        nonadditive: {}, // nothing evaluated this cycle
        positionCount: 12,
      },
    ]);
    // header + 1 node row + footer
    expect(spill).toHaveLength(3);
    expect(spill[0]).toEqual([
      "group",
      "count",
      "delta",
      "gamma",
      "vega",
      "theta",
      "vanna",
      "volga",
      "premium",
      "VaR",
      "ES",
      "curvature",
    ]);
    const row = spill[1]!;
    expect(row[0]).toBe("FIRM");
    expect(row[1]).toBe(12);
    expect(row[2]).toBe(1000);
    // VaR/ES/curvature absent ⇒ blank, never a spurious zero.
    expect(row[9]).toBe("");
    expect(row[10]).toBe("");
    expect(row[11]).toBe("");
    expect(String(spill[2]![0])).toMatch(/reporting USD/);
    expect(String(spill[2]![0])).toMatch(/server-aggregated/);
  });

  it("formats CELNET.POSITIONS with an honest empty-state", () => {
    const spill = formatPositionsSpill([]);
    expect(spill).toHaveLength(2);
    expect(String(spill[1]![0])).toMatch(/no entitled open positions/);
  });

  it("formats CELNET.LIMITS annotating a vega-bucket metric and a breach footer", () => {
    const spill = formatLimitsSpill(
      "DESK:99",
      [
        {
          metric: "VEGA_BUCKET",
          vegaPillar: { tenorDays: 365, deltaBp: 2500 },
          tenorDays: 0,
          cap: 100,
          exposure: 150,
          ratio: 1.5,
          status: "BREACH",
          enforcement: "HARD",
          headroom: -50,
        },
      ],
      "BREACH",
      true,
    );
    expect(spill).toHaveLength(3);
    expect(String(spill[1]![0])).toBe("VEGA_BUCKET(365d,2500bp)");
    expect(String(spill[2]![0])).toMatch(/worst BREACH \| HARD BREACH/);
  });
});
