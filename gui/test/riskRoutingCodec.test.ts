/**
 * Risk-routing wsCodec round-trip + wire-shape tests — the GUI end of the ONE
 * `celnet.wire` risk-routing contract (server phases 4-5). For every value with an
 * encoder + decoder, `fromWire(toWire(x)) === x`, and the ON-THE-WIRE JSON matches
 * the server codec byte-for-byte: snake_case names, NUMERIC enum tags
 * (`field`/`op`/`band`), variant-keyed oneofs (`RouteValue`, `RoutingNode`), and
 * OMIT-ABSENT for a book's `parent_id`/`desk_id`/`limits` + each `RiskLimits` cap.
 * `dv01`/`pnl` decode `null` → `null` (never a fabricated 0).
 */
import { describe, expect, it } from "vitest";

import {
  riskLimitsToWire,
  riskLimitsFromWire,
  riskBookSpecToWire,
  riskBookDescFromWire,
  routeValueToWire,
  routeValueFromWire,
  routeConditionToWire,
  routeConditionFromWire,
  routingNodeToWire,
  routingNodeFromWire,
  riskRoutingGraphToWire,
  riskRoutingGraphFromWire,
  riskBookRiskDescFromWire,
  routeFieldToWire,
  routeFieldFromWire,
  routeOpToWire,
  routeOpFromWire,
  ragBandFromWire,
  riskBooksResponseFromWire,
  riskBookRiskResponseFromWire,
  riskRoutingGraphResponseFromWire,
} from "../src/data/wsCodec";
import type {
  RiskBook,
  RiskLimits,
  RouteCondition,
  RouteField,
  RouteOp,
  RouteValue,
  RoutingNode,
  RiskRoutingGraph,
} from "../src/data/contract";

const ALL_FIELDS: RouteField[] = [
  "instrument_id",
  "ccy",
  "product",
  "side",
  "notional",
  "tenor",
  "strike",
  "counterparty",
  "user",
  "desk",
  "price",
];
const ALL_OPS: RouteOp[] = ["eq", "ne", "gt", "ge", "lt", "le", "contains", "in", "between"];

describe("risk-routing codec — enum ordinal maps", () => {
  it("maps every RouteField to its proto ordinal 0..10 and back", () => {
    ALL_FIELDS.forEach((f, i) => {
      expect(routeFieldToWire(f)).toBe(i);
      expect(routeFieldFromWire(i)).toBe(f);
    });
  });

  it("maps every RouteOp to its proto ordinal 0..8 and back", () => {
    ALL_OPS.forEach((op, i) => {
      expect(routeOpToWire(op)).toBe(i);
      expect(routeOpFromWire(i)).toBe(op);
    });
  });

  it("maps RagBand ordinals green=0 / amber=1 / red=2", () => {
    expect(ragBandFromWire(0)).toBe("green");
    expect(ragBandFromWire(1)).toBe("amber");
    expect(ragBandFromWire(2)).toBe("red");
  });

  it("clamps an out-of-range enum tag to the proto3 zero", () => {
    expect(routeFieldFromWire(99)).toBe("instrument_id");
    expect(routeOpFromWire(99)).toBe("eq");
    expect(ragBandFromWire(99)).toBe("green");
  });
});

describe("risk-routing codec — RiskLimits", () => {
  it("round-trips a fully-populated limits object", () => {
    const l: RiskLimits = { maxNetNotional: 1e9, maxGrossNotional: 2e9, maxDv01: 50000 };
    expect(riskLimitsFromWire(riskLimitsToWire(l))).toEqual(l);
  });

  it("OMITS each absent cap on the wire (null ≠ 0)", () => {
    const l: RiskLimits = { maxNetNotional: 400e6, maxGrossNotional: null, maxDv01: null };
    const w = riskLimitsToWire(l);
    expect(Object.keys(w)).toEqual(["max_net_notional"]);
    expect(riskLimitsFromWire(w)).toEqual(l);
  });
});

describe("risk-routing codec — RiskBook", () => {
  it("round-trips a book with a parent, desk and limits", () => {
    const b: RiskBook = {
      id: "fx-emea-vanilla",
      name: "FX EMEA Vanilla",
      parentId: "fx-emea",
      deskId: "emea",
      description: "EMEA vanilla sub-book",
      limits: { maxNetNotional: 4e8, maxGrossNotional: null, maxDv01: null },
      enabled: true,
      assetClass: "fx_options",
    };
    expect(riskBookDescFromWire(riskBookSpecToWire(b))).toEqual(b);
  });

  it("OMITS parent_id / desk_id / limits when absent (a top-level uncapped book)", () => {
    const b: RiskBook = {
      id: "fx-apac",
      name: "FX APAC",
      parentId: null,
      deskId: null,
      description: "",
      limits: null,
      enabled: false,
      assetClass: "fx_options",
    };
    const w = riskBookSpecToWire(b);
    expect("parent_id" in w).toBe(false);
    expect("desk_id" in w).toBe(false);
    expect("limits" in w).toBe(false);
    expect(riskBookDescFromWire(w)).toEqual(b);
  });
});

describe("risk-routing codec — RouteValue oneof", () => {
  const cases: RouteValue[] = [
    { kind: "num", num: 5_000_000 },
    { kind: "num", num: 0 },
    { kind: "text", text: "EURUSD" },
    { kind: "list", values: ["EUR", "GBP", "USD"] },
    { kind: "range", lo: 0.25, hi: 2.0 },
  ];

  it.each(cases)("round-trips the %s arm variant-keyed", (v) => {
    expect(routeValueFromWire(routeValueToWire(v))).toEqual(v);
  });

  it("emits exactly the live arm key", () => {
    expect(routeValueToWire({ kind: "num", num: 3 })).toEqual({ num: 3 });
    expect(routeValueToWire({ kind: "text", text: "x" })).toEqual({ text: "x" });
    expect(routeValueToWire({ kind: "list", values: ["a"] })).toEqual({ list: { values: ["a"] } });
    expect(routeValueToWire({ kind: "range", lo: 1, hi: 2 })).toEqual({ range: { lo: 1, hi: 2 } });
  });
});

describe("risk-routing codec — RouteCondition", () => {
  it("round-trips a numeric between-condition (enums as i32 tags)", () => {
    const c: RouteCondition = {
      field: "notional",
      op: "between",
      value: { kind: "range", lo: 1e6, hi: 1e8 },
      onTrue: 2,
      onFalse: 3,
    };
    const w = routeConditionToWire(c);
    expect(w["field"]).toBe(4);
    expect(w["op"]).toBe(8);
    expect(w["on_true"]).toBe(2);
    expect(w["on_false"]).toBe(3);
    expect(routeConditionFromWire(w)).toEqual(c);
  });

  it("carries a null value as JSON null and decodes it back to null", () => {
    const c: RouteCondition = { field: "ccy", op: "eq", value: null, onTrue: 1, onFalse: 0 };
    const w = routeConditionToWire(c);
    expect(w["value"]).toBeNull();
    expect(routeConditionFromWire(w)).toEqual(c);
  });
});

describe("risk-routing codec — RoutingNode oneof", () => {
  it("round-trips a condition node", () => {
    const n: RoutingNode = {
      kind: "condition",
      id: 1,
      condition: {
        field: "ccy",
        op: "in",
        value: { kind: "list", values: ["EUR", "USD"] },
        onTrue: 2,
        onFalse: 3,
      },
    };
    const w = routingNodeToWire(n);
    expect(w["id"]).toBe(1);
    expect("condition" in w).toBe(true);
    expect(routingNodeFromWire(w)).toEqual(n);
  });

  it("round-trips a terminal book leaf via book_risk_book_id", () => {
    const n: RoutingNode = { kind: "book", id: 2, bookId: "fx-emea" };
    const w = routingNodeToWire(n);
    expect(w).toEqual({ id: 2, book_risk_book_id: "fx-emea" });
    expect(routingNodeFromWire(w)).toEqual(n);
  });
});

describe("risk-routing codec — RiskRoutingGraph", () => {
  it("round-trips a multi-node decision graph (the pass-6b contract)", () => {
    const g: RiskRoutingGraph = {
      entry: 0,
      nodes: [
        {
          kind: "condition",
          id: 0,
          condition: {
            field: "notional",
            op: "ge",
            value: { kind: "num", num: 5e7 },
            onTrue: 1,
            onFalse: 2,
          },
        },
        { kind: "book", id: 1, bookId: "fx-emea" },
        { kind: "book", id: 2, bookId: "fx-emea-vanilla" },
      ],
    };
    expect(riskRoutingGraphFromWire(riskRoutingGraphToWire(g))).toEqual(g);
  });

  it("decodes an absent graph reply as null", () => {
    expect(riskRoutingGraphResponseFromWire({})).toBeNull();
    expect(riskRoutingGraphResponseFromWire({ graph: null })).toBeNull();
  });
});

describe("risk-routing codec — RiskBookRisk decode", () => {
  it("decodes a full risk row incl. a RAG limit strip", () => {
    const wire = {
      book_id: "fx-emea",
      name: "FX EMEA",
      net_notional: -1.2e8,
      gross_notional: 8e8,
      position_count: 12,
      delta: 3_000_000,
      gamma: 42_000,
      vega: 250_000,
      theta: -18_000,
      dv01: null,
      pnl: null,
      limits: [{ metric: "net_notional", used: 1.2e8, limit: 1e9, fraction: 0.12, band: 0 }],
    };
    const r = riskBookRiskDescFromWire(wire);
    expect(r.bookId).toBe("fx-emea");
    expect(r.netNotional).toBe(-1.2e8);
    expect(r.positionCount).toBe(12);
    expect(r.dv01).toBeNull();
    expect(r.pnl).toBeNull();
    expect(r.limits).toEqual([
      { metric: "net_notional", used: 1.2e8, limit: 1e9, fraction: 0.12, band: "green" },
    ]);
  });

  it("bands amber (1) and red (2) tags correctly", () => {
    const r = riskBookRiskDescFromWire({
      book_id: "b",
      name: "B",
      net_notional: 0,
      gross_notional: 0,
      position_count: 0,
      delta: 0,
      gamma: 0,
      vega: 0,
      theta: 0,
      dv01: 123.5,
      pnl: -9.5,
      limits: [
        { metric: "net_notional", used: 9e8, limit: 1e9, fraction: 0.9, band: 1 },
        { metric: "gross_notional", used: 1.1e9, limit: 1e9, fraction: 1.1, band: 2 },
      ],
    });
    expect(r.dv01).toBe(123.5);
    expect(r.pnl).toBe(-9.5);
    expect(r.limits.map((l) => l.band)).toEqual(["amber", "red"]);
  });
});

describe("risk-routing codec — roster responses", () => {
  it("decodes a `risk_books` roster (`{ books: [...] }`)", () => {
    const list = riskBooksResponseFromWire({
      books: [riskBookSpecToWire({
        id: "a",
        name: "A",
        parentId: null,
        deskId: null,
        description: "",
        limits: null,
        enabled: true,
        // carried explicitly so the round-trip actually covers the franchise field
        assetClass: "fx_options",
      })],
    });
    expect(list).toHaveLength(1);
    expect(list[0]?.id).toBe("a");
  });

  it("decodes a `risk_book_risk` roster (`{ books: [...] }`)", () => {
    const list = riskBookRiskResponseFromWire({ books: [] });
    expect(list).toEqual([]);
  });
});
