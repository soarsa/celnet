/**
 * scope reducer — path algebra (GW1-S2).
 *
 * The verification oracle is a DISJOINT reference reducer (`refReducer`) derived
 * from the breadcrumb spec by trivial list truncation/append — it does NOT import
 * the production `scopeReducer`, so a symmetric bug cannot pass (the FRTB-`0.75ρ`
 * circular-oracle lesson). We assert the production reducer EQUALS the reference
 * over a generated action space, plus the load-bearing invariants the plan calls
 * out: drill round-trip idempotence, pin order-independence, FX-terminal == pair,
 * and the group-by reconciliation on drill-up. Serialisation round-trips too.
 */

import { describe, expect, it } from "vitest";

import {
  childLevel,
  currentLevel,
  decodeScopePath,
  encodeScopePath,
  FIRM_SCOPE_ROOT,
  INITIAL_SCOPE,
  isTerminal,
  SCOPE_LADDER,
  scopeReducer,
  type ScopeAction,
  type ScopeGroupBy,
  type ScopeNode,
  type ScopeState,
} from "../src/lib/scope";

// --- the DISJOINT reference reducer (the oracle; never imports scopeReducer) ---

const LADDER = ["firm", "desk", "book", "pair"] as const;

/** Relax a finer-than-tail group-by to none (applied only when the tail moves up). */
function refRelax(path: ScopeState["path"], groupBy: ScopeGroupBy): ScopeGroupBy {
  if (groupBy === "none") return "none";
  const tailRank = LADDER.indexOf(path[path.length - 1]!.level);
  return LADDER.indexOf(groupBy) > tailRank ? "none" : groupBy;
}

/** A trivial truncation/append reference, independent of the production reducer. */
function refReducer(state: ScopeState, action: ScopeAction): ScopeState {
  if (action.type === "reset") return { path: [{ level: "firm", label: "Firm" }], groupBy: "none" };
  // Pinning persists verbatim (order-independent); the path is untouched.
  if (action.type === "setGroupBy") return { path: state.path, groupBy: action.groupBy };
  if (action.type === "drillDown") {
    const tail = state.path[state.path.length - 1]!.level;
    const idx = LADDER.indexOf(tail);
    if (idx >= LADDER.length - 1) return state; // terminal
    return {
      path: [...state.path, { level: LADDER[idx + 1]!, label: action.label }],
      groupBy: state.groupBy,
    };
  }
  // drillUp: keep the first `depth` crumbs (clamped). A no-op (depth == current
  // length) changes nothing; a real up-move relaxes a now-finer group-by.
  const depth = Math.max(1, Math.min(action.depth, state.path.length));
  if (depth === state.path.length) return state;
  const path = state.path.slice(0, depth);
  return { path, groupBy: refRelax(path, state.groupBy) };
}

/** Drill the path all the way down to a pair with descriptive labels. */
function fullDrill(): ScopeState {
  let s = INITIAL_SCOPE;
  s = scopeReducer(s, { type: "drillDown", label: "EMEA" }); // desk
  s = scopeReducer(s, { type: "drillDown", label: "VOL-1" }); // book
  s = scopeReducer(s, { type: "drillDown", label: "EUR/USD" }); // pair
  return s;
}

describe("scope ladder & predicates", () => {
  it("the FX-default ladder is firm → desk → book → pair", () => {
    expect(SCOPE_LADDER).toEqual(["firm", "desk", "book", "pair"]);
  });

  it("childLevel walks the ladder and terminates at pair", () => {
    expect(childLevel("firm")).toBe("desk");
    expect(childLevel("desk")).toBe("book");
    expect(childLevel("book")).toBe("pair");
    expect(childLevel("pair")).toBeNull();
  });

  it("isTerminal is true only at the pair leaf", () => {
    expect(isTerminal(INITIAL_SCOPE)).toBe(false);
    expect(isTerminal(fullDrill())).toBe(true);
  });
});

describe("scopeReducer == disjoint reference over a generated action space", () => {
  const actions: ScopeAction[] = [
    { type: "drillDown", label: "EMEA" },
    { type: "drillDown", label: "VOL-1" },
    { type: "drillDown", label: "EUR/USD" },
    { type: "drillDown", label: "overshoot" }, // no-op at terminal
    { type: "drillUp", depth: 1 },
    { type: "drillUp", depth: 2 },
    { type: "drillUp", depth: 99 }, // clamp
    { type: "setGroupBy", groupBy: "book" },
    { type: "setGroupBy", groupBy: "pair" },
    { type: "setGroupBy", groupBy: "none" },
    { type: "reset" },
  ];

  it("matches the reference for every single action from a deep state", () => {
    for (const a of actions) {
      const start = { ...fullDrill(), groupBy: "book" as ScopeGroupBy };
      expect(scopeReducer(start, a)).toEqual(refReducer(start, a));
    }
  });

  it("matches the reference along a random-ish action walk (no drift)", () => {
    // A deterministic pseudo-walk: index into the action list by a simple LCG.
    let seed = 12345;
    let prod = INITIAL_SCOPE;
    let ref = INITIAL_SCOPE;
    for (let i = 0; i < 200; i += 1) {
      seed = (seed * 1103515245 + 12345) & 0x7fffffff;
      const a = actions[seed % actions.length]!;
      prod = scopeReducer(prod, a);
      ref = refReducer(ref, a);
      expect(prod).toEqual(ref);
    }
  });
});

describe("path-algebra invariants (reached without the production reducer's internals)", () => {
  it("drill-down then drill-up to an ancestor is idempotent to that ancestor", () => {
    const deep = fullDrill();
    // Drill up to the desk (depth 2: [firm, desk]).
    const up = scopeReducer(deep, { type: "drillUp", depth: 2 });
    expect(up.path.map((n) => n.level)).toEqual(["firm", "desk"]);
    // Drilling up again to the same depth is a no-op (idempotent).
    expect(scopeReducer(up, { type: "drillUp", depth: 2 })).toEqual(up);
  });

  it("pinning a group-by is order-independent (commutes with a no-op drill)", () => {
    const base = scopeReducer(INITIAL_SCOPE, { type: "drillDown", label: "EMEA" });
    const a = scopeReducer(scopeReducer(base, { type: "setGroupBy", groupBy: "book" }), {
      type: "drillDown",
      label: "VOL-1",
    });
    const b = scopeReducer(scopeReducer(base, { type: "drillDown", label: "VOL-1" }), {
      type: "setGroupBy",
      groupBy: "book",
    });
    expect(a).toEqual(b);
  });

  it("a finer group-by relaxes to none when drilling up above it", () => {
    let s = scopeReducer(INITIAL_SCOPE, { type: "drillDown", label: "EMEA" }); // desk
    s = scopeReducer(s, { type: "drillDown", label: "VOL-1" }); // book
    s = scopeReducer(s, { type: "setGroupBy", groupBy: "book" });
    // Drill up to the firm root: a `book` group-by is now finer than the tail → none.
    const up = scopeReducer(s, { type: "drillUp", depth: 1 });
    expect(currentLevel(up)).toBe("firm");
    expect(up.groupBy).toBe("none");
  });

  it("a group-by at-or-above the tail survives a drill-up", () => {
    let s = scopeReducer(INITIAL_SCOPE, { type: "drillDown", label: "EMEA" }); // desk
    s = scopeReducer(s, { type: "drillDown", label: "VOL-1" }); // book
    s = scopeReducer(s, { type: "setGroupBy", groupBy: "desk" });
    // Drill up to the desk: `desk` group-by is at the tail → preserved.
    const up = scopeReducer(s, { type: "drillUp", depth: 2 });
    expect(currentLevel(up)).toBe("desk");
    expect(up.groupBy).toBe("desk");
  });

  it("FX terminal crumb is the pair (drill-down past pair is a no-op)", () => {
    const deep = fullDrill();
    const tail = deep.path[deep.path.length - 1]!;
    expect(tail.level).toBe("pair");
    expect(tail.label).toBe("EUR/USD");
    expect(scopeReducer(deep, { type: "drillDown", label: "GBP/USD" })).toEqual(deep);
  });
});

describe("scope path serialisation (round-trip + forward-compat)", () => {
  it("encode then decode is identity (firm root implicit)", () => {
    const path: ScopeNode[] = [
      FIRM_SCOPE_ROOT,
      { level: "desk", label: "EM Vol" },
      { level: "book", label: "LATAM Vol" },
      { level: "pair", label: "EUR/USD" },
    ];
    expect(decodeScopePath(encodeScopePath(path))).toEqual(path);
  });

  it("encodes the firm root as the empty token", () => {
    expect(encodeScopePath([FIRM_SCOPE_ROOT])).toBe("");
    expect(decodeScopePath("")).toEqual([FIRM_SCOPE_ROOT]);
  });

  it("labels with slashes survive the round-trip (raw token; URL escaping is the codec's job)", () => {
    const path: ScopeNode[] = [FIRM_SCOPE_ROOT, { level: "desk", label: "FX/Rates" }];
    const tok = encodeScopePath(path);
    // The scope token keeps labels VERBATIM (URLSearchParams escapes at the URL
    // layer — proven in savedViews.test). The raw round-trip is identity.
    expect(tok).toBe("desk:FX/Rates");
    expect(decodeScopePath(tok)).toEqual(path);
  });

  it("drops corrupt / out-of-order crumbs (forward-compatible, never throws)", () => {
    // An unknown level, a malformed crumb, and a non-descending walk are all skipped.
    expect(decodeScopePath("bogus:X")).toEqual([FIRM_SCOPE_ROOT]);
    expect(decodeScopePath("no-colon")).toEqual([FIRM_SCOPE_ROOT]);
    // A pair crumb that skips desk/book (not one level below firm) is dropped.
    expect(decodeScopePath("pair:EUR/USD")).toEqual([FIRM_SCOPE_ROOT]);
    // The first valid descending crumb is kept; an out-of-order tail is dropped.
    expect(decodeScopePath("desk:EMEA>pair:EUR/USD").map((n) => n.level)).toEqual([
      "firm",
      "desk",
    ]);
  });
});
