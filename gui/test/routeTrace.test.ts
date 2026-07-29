/**
 * routeTrace — the client risk-routing engine. These tests pin CLIENT/SERVER
 * PARITY: the same canonical truth table the Rust oracle
 * (`crates/celnet-risk-routing`) uses must produce the same landing book here, so
 * the canvas "trace a sample fill" lights up exactly the branch the server would
 * route. They also pin the validator against the same defects
 * `RiskRoutingGraph::validate` rejects.
 *
 * Truth table (docs/FI-RISK-ROUTING-REQUIREMENTS.md §8.6):
 *   ccy = EUR & notional > 50m          → BOOK-A
 *   product = swap & tenor ≥ 10         → BOOK-B
 *   counterparty in [HF-1, HF-2]        → BOOK-C
 *   else                                → DEFAULT
 */
import { describe, expect, it } from "vitest";

import type { RiskRoutingGraph, RoutingNode } from "../src/data/contract";
import {
  blankFill,
  enumeratePaths,
  evalOp,
  traceGraph,
  validateGraph,
  type SampleFill,
} from "../src/lib/routeTrace";

function fill(overrides: Partial<SampleFill> = {}): SampleFill {
  return { ...blankFill(), ...overrides };
}

/** The canonical four-rule graph, node-for-node the Rust router's truth table. */
function truthTableGraph(): RiskRoutingGraph {
  const nodes: RoutingNode[] = [
    {
      kind: "condition",
      id: 0,
      condition: {
        field: "ccy",
        op: "eq",
        value: { kind: "text", text: "EUR" },
        onTrue: 1,
        onFalse: 3,
      },
    },
    {
      kind: "condition",
      id: 1,
      condition: {
        field: "notional",
        op: "gt",
        value: { kind: "num", num: 50_000_000 },
        onTrue: 2,
        onFalse: 3,
      },
    },
    { kind: "book", id: 2, bookId: "BOOK-A" },
    {
      kind: "condition",
      id: 3,
      condition: {
        field: "product",
        op: "eq",
        value: { kind: "text", text: "swap" },
        onTrue: 4,
        onFalse: 6,
      },
    },
    {
      kind: "condition",
      id: 4,
      condition: {
        field: "tenor",
        op: "ge",
        value: { kind: "num", num: 10 },
        onTrue: 5,
        onFalse: 6,
      },
    },
    { kind: "book", id: 5, bookId: "BOOK-B" },
    {
      kind: "condition",
      id: 6,
      condition: {
        field: "counterparty",
        op: "in",
        value: { kind: "list", values: ["HF-1", "HF-2"] },
        onTrue: 7,
        onFalse: 8,
      },
    },
    { kind: "book", id: 7, bookId: "BOOK-C" },
    { kind: "book", id: 8, bookId: "DEFAULT" },
  ];
  return { entry: 0, nodes };
}

const KNOWN_BOOKS = new Set(["BOOK-A", "BOOK-B", "BOOK-C", "DEFAULT"]);

describe("evalOp — the RouteOp::eval port", () => {
  it("numeric ordering + equality on (num, num)", () => {
    const x = { kind: "num", num: 10 } as const;
    expect(evalOp("eq", x, { kind: "num", num: 10 })).toBe(true);
    expect(evalOp("gt", x, { kind: "num", num: 9 })).toBe(true);
    expect(evalOp("gt", x, { kind: "num", num: 10 })).toBe(false);
    expect(evalOp("ge", x, { kind: "num", num: 10 })).toBe(true);
    expect(evalOp("le", x, { kind: "num", num: 10 })).toBe(true);
  });

  it("between is inclusive on both endpoints", () => {
    const range = { kind: "range", lo: 5, hi: 10 } as const;
    expect(evalOp("between", { kind: "num", num: 5 }, range)).toBe(true);
    expect(evalOp("between", { kind: "num", num: 10 }, range)).toBe(true);
    expect(evalOp("between", { kind: "num", num: 4.999 }, range)).toBe(false);
    expect(evalOp("between", { kind: "num", num: 10.001 }, range)).toBe(false);
  });

  it("contains + in membership on text", () => {
    expect(
      evalOp("contains", { kind: "text", text: "EURUSD" }, { kind: "text", text: "EUR" }),
    ).toBe(true);
    expect(
      evalOp("in", { kind: "text", text: "HF-2" }, { kind: "list", values: ["HF-1", "HF-2"] }),
    ).toBe(true);
    expect(
      evalOp("in", { kind: "text", text: "HF-9" }, { kind: "list", values: ["HF-1", "HF-2"] }),
    ).toBe(false);
  });

  it("in on a numeric context parses list entries to numbers", () => {
    expect(evalOp("in", { kind: "num", num: 3 }, { kind: "list", values: ["1", "3", "nope"] })).toBe(
      true,
    );
    expect(evalOp("in", { kind: "num", num: 2 }, { kind: "list", values: ["1", "3"] })).toBe(false);
    expect(evalOp("in", { kind: "num", num: 0 }, { kind: "list", values: ["x"] })).toBe(false);
  });

  it("type mismatch is always false and never throws (total)", () => {
    expect(evalOp("gt", { kind: "text", text: "a" }, { kind: "num", num: 1 })).toBe(false);
    expect(evalOp("eq", { kind: "num", num: 1 }, { kind: "text", text: "1" })).toBe(false);
    expect(evalOp("ne", { kind: "num", num: 1 }, { kind: "text", text: "1" })).toBe(false);
    expect(evalOp("between", { kind: "num", num: 1 }, { kind: "num", num: 1 })).toBe(false);
  });
});

describe("traceGraph — server-parity routing of the canonical truth table", () => {
  const g = truthTableGraph();
  const cases: { name: string; fill: SampleFill; book: string }[] = [
    { name: "EUR & big notional → BOOK-A", fill: fill({ ccy: "EUR", notional: 60_000_000 }), book: "BOOK-A" },
    { name: "EUR but small notional → DEFAULT", fill: fill({ ccy: "EUR", notional: 10_000_000 }), book: "DEFAULT" },
    { name: "swap & long tenor → BOOK-B", fill: fill({ product: "swap", tenor: 10 }), book: "BOOK-B" },
    { name: "swap but short tenor → DEFAULT", fill: fill({ product: "swap", tenor: 2 }), book: "DEFAULT" },
    { name: "HF-1 counterparty → BOOK-C", fill: fill({ counterparty: "HF-1" }), book: "BOOK-C" },
    { name: "HF-2 counterparty → BOOK-C", fill: fill({ counterparty: "HF-2" }), book: "BOOK-C" },
    { name: "unmatched → DEFAULT", fill: fill({ ccy: "USD", counterparty: "BANK-X" }), book: "DEFAULT" },
  ];
  for (const c of cases) {
    it(c.name, () => {
      const r = traceGraph(g, c.fill);
      expect(r.outcome).toBe("book");
      expect(r.landedBook).toBe(c.book);
    });
  }

  it("collects the visited node path for the highlight", () => {
    const r = traceGraph(g, fill({ ccy: "EUR", notional: 60_000_000 }));
    expect(r.path).toEqual([0, 1, 2]);
  });

  it("a fill matching an earlier rule short-circuits before later rules", () => {
    // EUR + big notional AND also a swap: BOOK-A wins because it is tested first.
    const r = traceGraph(g, fill({ ccy: "EUR", notional: 99_000_000, product: "swap", tenor: 30 }));
    expect(r.landedBook).toBe("BOOK-A");
  });

  it("a cyclic graph terminates as `cycle`, never hangs", () => {
    const cyclic: RiskRoutingGraph = {
      entry: 0,
      nodes: [
        {
          kind: "condition",
          id: 0,
          condition: {
            field: "ccy",
            op: "eq",
            value: { kind: "text", text: "EUR" },
            onTrue: 0,
            onFalse: 0,
          },
        },
      ],
    };
    expect(traceGraph(cyclic, fill({ ccy: "EUR" })).outcome).toBe("cycle");
  });
});

describe("validateGraph — mirrors RiskRoutingGraph::validate", () => {
  it("accepts a well-formed graph (no issues)", () => {
    expect(validateGraph(truthTableGraph(), KNOWN_BOOKS)).toEqual([]);
  });

  it("flags a missing entry", () => {
    const g: RiskRoutingGraph = { entry: 99, nodes: [{ kind: "book", id: 0, bookId: "DEFAULT" }] };
    const issues = validateGraph(g, KNOWN_BOOKS);
    expect(issues.some((i) => i.code === "missing_entry")).toBe(true);
  });

  it("flags a dangling on_true/on_false edge", () => {
    const g: RiskRoutingGraph = {
      entry: 0,
      nodes: [
        {
          kind: "condition",
          id: 0,
          condition: {
            field: "ccy",
            op: "eq",
            value: { kind: "text", text: "EUR" },
            onTrue: 77,
            onFalse: 1,
          },
        },
        { kind: "book", id: 1, bookId: "DEFAULT" },
      ],
    };
    const issues = validateGraph(g, KNOWN_BOOKS);
    expect(issues.some((i) => i.code === "dangling_edge" && i.node === 0)).toBe(true);
  });

  it("flags a book leaf targeting an unknown book", () => {
    const g: RiskRoutingGraph = { entry: 0, nodes: [{ kind: "book", id: 0, bookId: "NOPE" }] };
    expect(validateGraph(g, KNOWN_BOOKS).some((i) => i.code === "unknown_book")).toBe(true);
  });

  it("flags an operator not valid for the field kind", () => {
    const g: RiskRoutingGraph = {
      entry: 0,
      nodes: [
        {
          kind: "condition",
          id: 0,
          // `contains` is illegal on a numeric field.
          condition: {
            field: "notional",
            op: "contains",
            value: { kind: "text", text: "x" },
            onTrue: 1,
            onFalse: 1,
          },
        },
        { kind: "book", id: 1, bookId: "DEFAULT" },
      ],
    };
    expect(validateGraph(g, KNOWN_BOOKS).some((i) => i.code === "op_not_valid_for_field")).toBe(true);
  });

  it("flags a value variant inconsistent with the operator", () => {
    const g: RiskRoutingGraph = {
      entry: 0,
      nodes: [
        {
          kind: "condition",
          id: 0,
          // `between` needs a range, not a bare number.
          condition: {
            field: "notional",
            op: "between",
            value: { kind: "num", num: 1 },
            onTrue: 1,
            onFalse: 1,
          },
        },
        { kind: "book", id: 1, bookId: "DEFAULT" },
      ],
    };
    expect(validateGraph(g, KNOWN_BOOKS).some((i) => i.code === "value_type_mismatch")).toBe(true);
  });

  it("flags a reachable cycle", () => {
    const g: RiskRoutingGraph = {
      entry: 0,
      nodes: [
        {
          kind: "condition",
          id: 0,
          condition: {
            field: "ccy",
            op: "eq",
            value: { kind: "text", text: "EUR" },
            onTrue: 1,
            onFalse: 1,
          },
        },
        {
          kind: "condition",
          id: 1,
          condition: {
            field: "ccy",
            op: "eq",
            value: { kind: "text", text: "USD" },
            onTrue: 0,
            onFalse: 0,
          },
        },
      ],
    };
    expect(validateGraph(g, KNOWN_BOOKS).some((i) => i.code === "cycle")).toBe(true);
  });

  it("flags an unset condition value", () => {
    const g: RiskRoutingGraph = {
      entry: 0,
      nodes: [
        {
          kind: "condition",
          id: 0,
          condition: { field: "ccy", op: "eq", value: null, onTrue: 1, onFalse: 1 },
        },
        { kind: "book", id: 1, bookId: "DEFAULT" },
      ],
    };
    expect(validateGraph(g, KNOWN_BOOKS).some((i) => i.code === "value_unset")).toBe(true);
  });
});

describe("enumeratePaths — every rule in evaluation order", () => {
  /** ccy == EUR → BOOK-A, else → DEFAULT. Two paths, true-branch first. */
  function twoRuleGraph(): RiskRoutingGraph {
    return {
      entry: 0,
      nodes: [
        {
          kind: "condition",
          id: 0,
          condition: { field: "ccy", op: "eq", value: { kind: "text", text: "EUR" }, onTrue: 1, onFalse: 2 },
        },
        { kind: "book", id: 1, bookId: "BOOK-A" },
        { kind: "book", id: 2, bookId: "DEFAULT" },
      ],
    };
  }

  it("enumerates root-to-leaf paths in on_true-before-on_false order", () => {
    const paths = enumeratePaths(twoRuleGraph(), KNOWN_BOOKS);
    expect(paths.map((p) => p.bookId)).toEqual(["BOOK-A", "DEFAULT"]);
    expect(paths.every((p) => p.valid)).toBe(true);
    // Rule 1 = the EUR match (on_true); Rule 2 = its negation (on_false).
    expect(paths[0]?.conditions).toEqual([
      { field: "ccy", op: "eq", value: { kind: "text", text: "EUR" }, branch: "onTrue" },
    ]);
    expect(paths[1]?.conditions[0]?.branch).toBe("onFalse");
    expect(paths[0]?.nodes).toEqual([0, 1]);
  });

  it("enumerates the canonical four-rule truth table with each leaf reachable", () => {
    const paths = enumeratePaths(truthTableGraph(), KNOWN_BOOKS);
    // First rule is the EUR & big-notional path landing BOOK-A.
    expect(paths[0]?.bookId).toBe("BOOK-A");
    expect(paths.every((p) => p.valid)).toBe(true);
    // Every canonical book is reached by at least one rule.
    const landings = new Set(paths.map((p) => p.bookId));
    for (const b of ["BOOK-A", "BOOK-B", "BOOK-C", "DEFAULT"]) expect(landings.has(b)).toBe(true);
  });

  it("flags a path that dangles off a missing node as invalid", () => {
    const g: RiskRoutingGraph = {
      entry: 0,
      nodes: [
        {
          kind: "condition",
          id: 0,
          condition: { field: "ccy", op: "eq", value: { kind: "text", text: "EUR" }, onTrue: 77, onFalse: 1 },
        },
        { kind: "book", id: 1, bookId: "DEFAULT" },
      ],
    };
    const paths = enumeratePaths(g, KNOWN_BOOKS);
    // The on_true leg dangles (invalid, no book); the on_false leg reaches DEFAULT.
    const bad = paths.find((p) => !p.valid);
    expect(bad?.bookId).toBeNull();
    expect(bad?.issue).toMatch(/#77/);
    expect(paths.some((p) => p.valid && p.bookId === "DEFAULT")).toBe(true);
  });

  it("flags a leaf targeting an unknown / disabled book as invalid", () => {
    const g: RiskRoutingGraph = { entry: 0, nodes: [{ kind: "book", id: 0, bookId: "NOPE" }] };
    const paths = enumeratePaths(g, KNOWN_BOOKS);
    expect(paths).toHaveLength(1);
    expect(paths[0]?.valid).toBe(false);
    expect(paths[0]?.issue).toMatch(/unknown/i);
  });

  it("flags a cyclic path as invalid without hanging", () => {
    const cyclic: RiskRoutingGraph = {
      entry: 0,
      nodes: [
        {
          kind: "condition",
          id: 0,
          condition: { field: "ccy", op: "eq", value: { kind: "text", text: "EUR" }, onTrue: 1, onFalse: 1 },
        },
        {
          kind: "condition",
          id: 1,
          condition: { field: "ccy", op: "eq", value: { kind: "text", text: "USD" }, onTrue: 0, onFalse: 0 },
        },
      ],
    };
    const paths = enumeratePaths(cyclic, KNOWN_BOOKS);
    expect(paths.length).toBeGreaterThan(0);
    expect(paths.every((p) => !p.valid)).toBe(true);
    expect(paths.some((p) => /loops/i.test(p.issue ?? ""))).toBe(true);
  });
});
