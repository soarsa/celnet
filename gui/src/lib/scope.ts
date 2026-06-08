/**
 * scope.ts — the GW1 scope grammar: the FX-default dimension ladder plus the
 * pure drill-up / drill-down / pin reducer the breadcrumb composes from.
 *
 * The scope answers "WHAT slice of the firm am I looking at": an ordered drill
 * path from the firm root down to (in the FX-default world) a currency pair —
 * the TERMINAL crumb is underlier selection. This generalises the old
 * `AppContext.ScopeLevel` (firm|desk|book|pair) and the drill-UP-only
 * `ScopeBreadcrumb` into one reducer that drills BOTH ways and supports a
 * secondary group-by axis + pinning.
 *
 * DESIGN (GW-FOUNDATION-PLAN §2, scope-drill row): this module is the
 * IMPLEMENTATION. The verification oracle in `test/scope.test.ts` re-derives the
 * same path algebra from a trivial, DISJOINT list-truncation/append reference
 * (NOT importing this reducer) so a symmetric bug cannot pass — the FRTB-`0.75ρ`
 * circular-oracle lesson applied to the GUI.
 *
 * FX-default (no multi-asset axis — that is GW6): the ladder ends at `pair`. The
 * ladder is a DATA list, so a future asset-class axis is an entry here, never a
 * rewrite.
 */

/**
 * The organisational dimension a scope crumb sits at, firm-down. The FX-default
 * ladder; the tail is the active scope. `pair` is the terminal (underlier) crumb.
 */
export type ScopeLevel = "firm" | "desk" | "book" | "pair";

/**
 * The FX-default dimension ladder, root-first. The order is load-bearing: drill
 * DOWN moves to the next level after the current tail; drill UP truncates back to
 * an ancestor. `pair` is terminal — the underlier-selection crumb.
 */
export const SCOPE_LADDER: readonly ScopeLevel[] = ["firm", "desk", "book", "pair"] as const;

/** One node on the scope path (a breadcrumb crumb): its level + a human label. */
export interface ScopeNode {
  level: ScopeLevel;
  label: string;
}

/**
 * A secondary grouping dimension for scoped/aggregated views. `none` (no extra
 * grouping) plus each non-firm ladder level the rolled-up tree can be sliced by.
 */
export type ScopeGroupBy = "none" | "desk" | "book" | "pair";

/** The selectable group-by axes, in menu order (`none` first). */
export const SCOPE_GROUP_BY: readonly ScopeGroupBy[] = ["none", "desk", "book", "pair"] as const;

/** The firm root — "all desks · all books · all pairs". Every path starts here. */
export const FIRM_SCOPE_ROOT: ScopeNode = { level: "firm", label: "Firm" };

/**
 * The scope STATE: the drill `path` (firm root first; tail = current scope) and
 * the secondary `groupBy`. The entitlement principal lives in `AppContext` (the
 * GUI is grant-all today); this module is the pure path+groupBy algebra.
 */
export interface ScopeState {
  /** The drill path, firm root first. The tail crumb is the current scope. */
  path: ScopeNode[];
  /** The secondary grouping axis for aggregated views. */
  groupBy: ScopeGroupBy;
}

/** The initial scope: firm root, no secondary grouping. */
export const INITIAL_SCOPE: ScopeState = { path: [FIRM_SCOPE_ROOT], groupBy: "none" };

/** The next ladder level below `level`, or `null` if `level` is terminal (`pair`). */
export function childLevel(level: ScopeLevel): ScopeLevel | null {
  const i = SCOPE_LADDER.indexOf(level);
  if (i < 0 || i >= SCOPE_LADDER.length - 1) return null;
  return SCOPE_LADDER[i + 1]!;
}

/** The current scope crumb: the tail of the path (always defined — root never empties). */
export function currentNode(state: ScopeState): ScopeNode {
  return state.path[state.path.length - 1] ?? FIRM_SCOPE_ROOT;
}

/** The level of the current scope (the tail crumb's level). */
export function currentLevel(state: ScopeState): ScopeLevel {
  return currentNode(state).level;
}

/** `true` iff the current scope is terminal (a pair) — no further drill-down. */
export function isTerminal(state: ScopeState): boolean {
  return childLevel(currentLevel(state)) === null;
}

/**
 * The scope actions the breadcrumb dispatches. A discriminated union so the
 * reducer is total and the test's disjoint reference can mirror the same surface.
 */
export type ScopeAction =
  | { type: "drillDown"; label: string }
  | { type: "drillUp"; depth: number }
  | { type: "reset" }
  | { type: "setGroupBy"; groupBy: ScopeGroupBy };

/**
 * The pure scope reducer.
 *
 *  - `drillDown` appends a crumb one ladder level BELOW the current tail (a no-op
 *    at the terminal `pair` level — there is nothing below an underlier).
 *  - `drillUp` truncates the path to `depth` crumbs (clamped to `[1, len]`, so the
 *    firm root is never removed and over-deep is a no-op). Drilling up to an
 *    ancestor that is no longer the group-by source RELAXES `groupBy` to `none`
 *    when the pinned axis is below the new tail (you cannot group by a dimension
 *    finer than where you stand).
 *  - `reset` returns to the firm root with no grouping.
 *  - `setGroupBy` pins the secondary axis (order-independent — it never touches
 *    the path).
 *
 * The reducer NEVER mutates its input (new arrays/objects), so React state and
 * the test reference compare by value cleanly.
 */
export function scopeReducer(state: ScopeState, action: ScopeAction): ScopeState {
  switch (action.type) {
    case "drillDown": {
      const next = childLevel(currentLevel(state));
      if (next === null) return state; // terminal — nothing below a pair.
      return {
        path: [...state.path, { level: next, label: action.label }],
        groupBy: state.groupBy,
      };
    }
    case "drillUp": {
      const depth = Math.max(1, Math.min(action.depth, state.path.length));
      if (depth === state.path.length) return state;
      const path = state.path.slice(0, depth);
      return { path, groupBy: reconcileGroupBy(path, state.groupBy) };
    }
    case "reset":
      return INITIAL_SCOPE;
    case "setGroupBy":
      // Pinning persists the user's INTENT verbatim (order-independent — it never
      // touches the path and is not relaxed here). A finer-than-tail axis is relaxed
      // only when a drill-up actually moves the tail above it (see `drillUp`).
      if (action.groupBy === state.groupBy) return state;
      return { path: state.path, groupBy: action.groupBy };
  }
}

/**
 * Keep the group-by axis coherent with the path after a drill-up: a group-by axis
 * STRICTLY FINER than the new tail level is no longer meaningful (you cannot slice
 * by `book` when you have drilled up to `firm`), so it relaxes to `none`. A
 * group-by at-or-above the tail is preserved.
 */
function reconcileGroupBy(path: ScopeNode[], groupBy: ScopeGroupBy): ScopeGroupBy {
  if (groupBy === "none") return "none";
  const tail = path[path.length - 1]?.level ?? "firm";
  const tailRank = SCOPE_LADDER.indexOf(tail);
  const groupRank = SCOPE_LADDER.indexOf(groupBy);
  // A finer group-by (deeper in the ladder than the tail) is dropped.
  return groupRank > tailRank ? "none" : groupBy;
}

// --- serialisation (consumed by lib/savedViews.ts) --------------------------

/**
 * Encode a scope path to a compact RAW token: `level:label` crumbs joined by `>`,
 * with labels kept verbatim — URL-level escaping is applied by the SAVED-VIEW codec
 * via `URLSearchParams` (which encodes `:` → `%3A`, `>` → `%3E`, space → `+`,
 * `/` → `%2F`), so we do NOT double-encode here. The firm root is implicit (always
 * crumb 0) and OMITTED, so an empty token decodes to the firm root.
 * e.g. `[firm, desk:EM Vol, pair:EUR/USD]` → `desk:EM Vol>pair:EUR/USD`.
 */
export function encodeScopePath(path: ScopeNode[]): string {
  return path
    .slice(1) // firm root is implicit
    .map((n) => `${n.level}:${n.label}`)
    .join(">");
}

/**
 * Decode a RAW scope-path token (post URL-decoding by the saved-view codec) back to
 * a path (firm root prepended). Unknown levels and malformed crumbs are SKIPPED
 * (forward-compatible — never throws); an empty token yields the bare firm root.
 * The decoded path is clamped to a single monotonically-descending ladder walk so a
 * corrupt token can't produce an out-of-order path.
 */
export function decodeScopePath(token: string): ScopeNode[] {
  const path: ScopeNode[] = [FIRM_SCOPE_ROOT];
  if (token.trim().length === 0) return path;
  for (const crumb of token.split(">")) {
    const sep = crumb.indexOf(":");
    if (sep < 0) continue;
    const level = crumb.slice(0, sep);
    const label = crumb.slice(sep + 1);
    if (!isScopeLevel(level)) continue;
    const expected = childLevel(path[path.length - 1]!.level);
    // The next crumb must be exactly one level below the current tail (a clean
    // descending walk); anything else is a corrupt token and is dropped.
    if (level !== expected) continue;
    path.push({ level, label });
  }
  return path;
}

/** Type guard: is `s` one of the ladder levels? */
export function isScopeLevel(s: string): s is ScopeLevel {
  return (SCOPE_LADDER as readonly string[]).includes(s);
}

/** Type guard: is `s` a valid group-by axis? */
export function isScopeGroupBy(s: string): s is ScopeGroupBy {
  return (SCOPE_GROUP_BY as readonly string[]).includes(s);
}
