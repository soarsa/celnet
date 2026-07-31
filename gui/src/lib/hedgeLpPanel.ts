/**
 * Pure client-side mirror of the server standing-hedging-LP-panel resolver
 * (`celnet_hedge_routing::HedgeLpPanel::effective_lps` +
 * `crate::config::hedge_policy::HedgeConfigDef::resolve_lp_panel`, committed 12e08c8).
 *
 * The include/exclude selection every EXTERNAL hedge action inherits:
 *   - base = {@link HedgeLpPanel.include} (in order) when non-empty, else ALL known LPs;
 *   - then subtract {@link HedgeLpPanel.exclude};
 *   - dedup preserving order.
 * The server rejects unknown ids and any panel whose effective set is empty — the same
 * two checks {@link validateLpPanel} surfaces in the editor before a save is attempted.
 *
 * Scope resolution is MOST-SPECIFIC-WINS: instrument > book > desk (mirrors the server
 * `resolve_lp_panel`), so a hedge on `(instrument, book, desk)` inherits the tightest
 * panel that names any of them.
 */
import type { HedgeLpPanel, HedgeScopeKind } from "../data/contract";

/**
 * The known liquidity-provider set the hedge panel picks from — the advisory LP roster
 * the app already exposes (the same `LP-1 … LP-4` the per-rule `RFQ_OUT` include picker
 * uses). Wiring this to the live aggregation / LP-session registry is a follow-up.
 */
export const KNOWN_LPS: readonly string[] = ["LP-1", "LP-2", "LP-3", "LP-4"];

/**
 * The effective LP set a panel resolves to against a known-LP roster: include-or-all,
 * minus excludes, deduped in order. Pure — the exact set the resolved-set display shows
 * and the mock threads onto advisory intents/provenance.
 */
export function effectiveLps(panel: HedgeLpPanel, knownLps: readonly string[] = KNOWN_LPS): string[] {
  const base = panel.include.length > 0 ? panel.include : [...knownLps];
  const excluded = new Set(panel.exclude);
  const seen = new Set<string>();
  const out: string[] = [];
  for (const lp of base) {
    if (excluded.has(lp) || seen.has(lp)) continue;
    seen.add(lp);
    out.push(lp);
  }
  return out;
}

/**
 * Validate a panel exactly as the server write boundary does: every include/exclude id
 * must be a known LP, and the effective set must be non-empty. Returns a (possibly empty)
 * list of human-readable defects; an empty list means the panel would be accepted.
 */
export function validateLpPanel(panel: HedgeLpPanel, knownLps: readonly string[] = KNOWN_LPS): string[] {
  const known = new Set(knownLps);
  const errs: string[] = [];
  for (const lp of panel.include) if (!known.has(lp)) errs.push(`Unknown LP in include: ${lp}`);
  for (const lp of panel.exclude) if (!known.has(lp)) errs.push(`Unknown LP in exclude: ${lp}`);
  if (effectiveLps(panel, knownLps).length === 0) {
    errs.push("Effective LP set is empty — a hedge would have no LP to fan to.");
  }
  return errs;
}

/** The scope a hedge fires for — any of which a panel may name. */
export interface HedgeScopeRef {
  instrument?: string;
  book?: string;
  desk?: string;
}

/**
 * Resolve the standing panel that governs a hedge scope, most-specific-wins
 * (instrument > book > desk). Returns `null` when no panel names any of the scope's
 * ids — the caller then falls back to the per-rule `RFQ_OUT` include list / full panel.
 */
export function resolveLpPanelForScope(
  panels: readonly HedgeLpPanel[],
  scope: HedgeScopeRef,
): HedgeLpPanel | null {
  const pick = (kind: HedgeScopeKind, id: string | undefined): HedgeLpPanel | undefined =>
    id === undefined ? undefined : panels.find((p) => p.scopeKind === kind && p.scopeId === id);
  return pick("instrument", scope.instrument) ?? pick("book", scope.book) ?? pick("desk", scope.desk) ?? null;
}
