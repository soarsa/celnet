/**
 * routingGuard — the pure predicate behind the startup "no default risk portfolio"
 * guard. These tests pin the whole truth table over the routing graph's catch-all
 * leaf: no graph → warn; a catch-all pointing at a disabled / missing / unset book →
 * warn; a catch-all pointing at an ENABLED book → ok; and that a NESTED on_false chain
 * resolves the TRUE terminal leaf (not an intermediate one). The graphs are built with
 * the shipped `compileRulesToGraph` so the predicate is exercised against real,
 * server-shaped graphs, plus one hand-wired dangling graph for the no-catch-all case.
 */
import { describe, expect, it } from "vitest";

import type { RiskBook, RiskRoutingGraph, RouteValue } from "../src/data/contract";
import {
  compileRulesToGraph,
  newRuleId,
  type RiskRule,
  type RuleCondition,
} from "../src/lib/riskRules";
import { resolveDefaultRoutedBook } from "../src/lib/routingGuard";

function cond(field: RuleCondition["field"], op: RuleCondition["op"], value: RouteValue): RuleCondition {
  return { field, op, value };
}
function rule(conditions: RuleCondition[], bookId: string | null): RiskRule {
  return { id: newRuleId(), conditions, bookId, enabled: true };
}
function book(id: string, enabled: boolean): RiskBook {
  return {
    id,
    name: id.toUpperCase(),
    parentId: null,
    deskId: null,
    description: "",
    limits: null,
    enabled,
    assetClass: "fx_options", // asset-class-agnostic fixture; a book must name ONE franchise
  };
}

const EUR = (): RouteValue => ({ kind: "text", text: "EUR" });

describe("resolveDefaultRoutedBook", () => {
  it("no graph installed ⇒ warn (no-graph)", () => {
    const r = resolveDefaultRoutedBook(null, [book("catchall", true)]);
    expect(r.ok).toBe(false);
    expect(r.reason).toBe("no-graph");
  });

  it("an empty graph ⇒ warn (no-graph)", () => {
    const r = resolveDefaultRoutedBook({ entry: 0, nodes: [] }, [book("catchall", true)]);
    expect(r.ok).toBe(false);
    expect(r.reason).toBe("no-graph");
  });

  it("catch-all → an ENABLED book ⇒ ok (silent)", () => {
    const graph = compileRulesToGraph([
      rule([cond("ccy", "eq", EUR())], "eur-book"),
      rule([], "catchall"), // default
    ]);
    const r = resolveDefaultRoutedBook(graph, [book("eur-book", true), book("catchall", true)]);
    expect(r.ok).toBe(true);
    expect(r.reason).toBe("ok");
    expect(r.bookId).toBe("catchall");
    expect(r.bookName).toBe("CATCHALL");
  });

  it("catch-all → a DISABLED book ⇒ warn (book-disabled)", () => {
    const graph = compileRulesToGraph([
      rule([cond("ccy", "eq", EUR())], "eur-book"),
      rule([], "catchall"),
    ]);
    const r = resolveDefaultRoutedBook(graph, [book("eur-book", true), book("catchall", false)]);
    expect(r.ok).toBe(false);
    expect(r.reason).toBe("book-disabled");
    expect(r.bookId).toBe("catchall");
    expect(r.bookName).toBe("CATCHALL");
  });

  it("catch-all → a book NOT in the roster ⇒ warn (book-not-found)", () => {
    const graph = compileRulesToGraph([
      rule([cond("ccy", "eq", EUR())], "eur-book"),
      rule([], "ghost-book"),
    ]);
    const r = resolveDefaultRoutedBook(graph, [book("eur-book", true)]);
    expect(r.ok).toBe(false);
    expect(r.reason).toBe("book-not-found");
    expect(r.bookId).toBe("ghost-book");
  });

  it("catch-all with no destination set ⇒ warn (empty-book)", () => {
    const graph = compileRulesToGraph([
      rule([cond("ccy", "eq", EUR())], "eur-book"),
      rule([], null), // catch-all with no book chosen
    ]);
    const r = resolveDefaultRoutedBook(graph, [book("eur-book", true)]);
    expect(r.ok).toBe(false);
    expect(r.reason).toBe("empty-book");
  });

  it("a NESTED on_false chain resolves the TRUE terminal leaf (the deep default)", () => {
    // Three conditional rules then the catch-all: the false spine walks past all three
    // to the trailing default. The predicate must resolve `deep-default`, NOT any of
    // the earlier rule destinations.
    const graph = compileRulesToGraph([
      rule([cond("ccy", "eq", { kind: "text", text: "EUR" })], "b1"),
      rule([cond("product", "eq", { kind: "text", text: "bond" })], "b2"),
      rule([cond("notional", "gt", { kind: "num", num: 5e7 })], "b3"),
      rule([], "deep-default"),
    ]);
    const books = [
      book("b1", true),
      book("b2", true),
      book("b3", true),
      book("deep-default", true),
    ];
    const r = resolveDefaultRoutedBook(graph, books);
    expect(r.ok).toBe(true);
    expect(r.bookId).toBe("deep-default");
  });

  it("a graph whose false spine dangles (no catch-all leaf) ⇒ warn (no-default)", () => {
    // One condition whose on_false points at a non-existent node — the walk falls off
    // the end without reaching an unconditional leaf.
    const graph: RiskRoutingGraph = {
      entry: 0,
      nodes: [
        {
          kind: "condition",
          id: 0,
          condition: { field: "ccy", op: "eq", value: EUR(), onTrue: 1, onFalse: 99 },
        },
        { kind: "book", id: 1, bookId: "eur-book" },
      ],
    };
    const r = resolveDefaultRoutedBook(graph, [book("eur-book", true)]);
    expect(r.ok).toBe(false);
    expect(r.reason).toBe("no-default");
  });
});
