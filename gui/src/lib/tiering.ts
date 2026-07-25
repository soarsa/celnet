/**
 * Pure helpers for the outbound price-tiering config (FI-TIERING phase 3): the
 * option vocabularies + human labels the editor renders, sensible defaults for a
 * freshly enabled config, and the client-side validator that MIRRORS the server's
 * `celnet-tiering` guardrail invariants (see `docs/FI-TIERING-RESEARCH.md` §4/§5).
 *
 * No React, no transport — so the Shell, the editor, and the vitest suite share
 * one source of truth. The server still enforces every invariant authoritatively;
 * this layer only surfaces inline errors and blocks a hopeless submit early.
 */

import type {
  TieringConfig,
  TieringGuardrails,
  TieringSpreadUnit,
  TieringStalePolicy,
  TieringStrategy,
  TieringStrategyKind,
} from "../data/contract";

// --- option vocabularies + labels (the editor's selects/toggles) -------------

/** The spread units, in wire (enum) order — the select's option order. */
export const TIERING_SPREAD_UNITS: readonly TieringSpreadUnit[] = [
  "PRICE_BPS",
  "YIELD_BPS",
  "PRICE_POINTS",
  "PERCENT",
];

/** Human labels for each spread unit. */
export const TIERING_SPREAD_UNIT_LABEL: Record<TieringSpreadUnit, string> = {
  PRICE_BPS: "Price bps",
  YIELD_BPS: "Yield bps",
  PRICE_POINTS: "Price points",
  PERCENT: "Percent",
};

/** The stale policies, in wire (enum) order. */
export const TIERING_STALE_POLICIES: readonly TieringStalePolicy[] = ["SUPPRESS", "WIDEN_TO_MAX"];

/** Human labels for each stale policy. */
export const TIERING_STALE_POLICY_LABEL: Record<TieringStalePolicy, string> = {
  SUPPRESS: "Suppress — publish no quote",
  WIDEN_TO_MAX: "Widen to max half-spread",
};

/** The strategy kinds shipped in phase 2a (the "add strategy" menu). */
export const TIERING_STRATEGY_KINDS: readonly TieringStrategyKind[] = [
  "FLAT_MARKUP",
  "INVENTORY_SKEW",
];

/** Human labels for each strategy kind. */
export const TIERING_STRATEGY_KIND_LABEL: Record<TieringStrategyKind, string> = {
  FLAT_MARKUP: "Flat markup",
  INVENTORY_SKEW: "Inventory skew",
};

/** A one-line description of what each strategy does (rendered under its row). */
export const TIERING_STRATEGY_KIND_HINT: Record<TieringStrategyKind, string> = {
  FLAT_MARKUP: "Constant symmetric half-spread H around mid — no skew.",
  INVENTORY_SKEW: "Base half-spread H plus clamp(κ·inventory, ±sMax) skew.",
};

// --- defaults ----------------------------------------------------------------

/**
 * The default guardrail bounds (absolute price offsets, points): a zero floor on
 * the half-spread, a generous 1.0-point cap, a 0.5-point skew cap, and a small
 * positive minimum tradeable spread. The admin tunes these per book.
 */
export const DEFAULT_TIERING_GUARDRAILS: TieringGuardrails = {
  hMin: 0,
  hMax: 1,
  sMax: 0.5,
  spreadFloor: 0.01,
};

/** A fresh strategy of `kind` with sensible starting magnitudes (in the config unit). */
export function defaultTieringStrategy(kind: TieringStrategyKind): TieringStrategy {
  return kind === "INVENTORY_SKEW"
    ? { kind, halfSpread: 25, kappa: 0.5, sMax: 0.5 }
    : { kind, halfSpread: 25, kappa: 0, sMax: 0 };
}

/**
 * The default config a newly ENABLED book gets: a single Flat-markup strategy at
 * 25 price bps (the worked example: mid 99.55 → ±0.25 → 99.30/99.80), the default
 * guardrails, and the safe `SUPPRESS` stale policy.
 */
export function defaultTieringConfig(): TieringConfig {
  return {
    unit: "PRICE_BPS",
    strategies: [defaultTieringStrategy("FLAT_MARKUP")],
    guardrails: { ...DEFAULT_TIERING_GUARDRAILS },
    stalePolicy: "SUPPRESS",
  };
}

// --- validation (mirrors the server's guardrail invariants) ------------------

/** Per-field errors for one strategy row (absent key ⇒ that field is valid). */
export interface TieringStrategyErrors {
  halfSpread?: string;
  kappa?: string;
  sMax?: string;
}

/** The structured error set for a whole tiering config (all-empty ⇒ valid). */
export interface TieringErrors {
  /** A config-level error (e.g. no strategies enabled). */
  form?: string;
  /** Per-strategy errors, keyed by index. */
  strategies: Record<number, TieringStrategyErrors>;
  /** Guardrail-field errors. */
  guardrails: {
    hMin?: string;
    hMax?: string;
    sMax?: string;
    spreadFloor?: string;
  };
}

/** A finite (not NaN/±∞) number guard used across the validator. */
function isFinite(x: number): boolean {
  return Number.isFinite(x);
}

/**
 * Validate a tiering config against the server's invariants: every magnitude
 * finite; `hMin ≥ 0`; `hMax ≥ hMin`; skew `sMax ≥ 0`; `spreadFloor > 0`; at least
 * one strategy; and each strategy's magnitudes finite/non-negative (κ may be any
 * finite value — a skew can lean either way). Returns a structured error set;
 * {@link hasTieringErrors} collapses it to a boolean.
 */
export function validateTiering(config: TieringConfig): TieringErrors {
  const errors: TieringErrors = { strategies: {}, guardrails: {} };

  if (config.strategies.length === 0) {
    errors.form = "Add at least one strategy (or disable tiering).";
  }

  config.strategies.forEach((s, i) => {
    const se: TieringStrategyErrors = {};
    if (!isFinite(s.halfSpread) || s.halfSpread < 0) {
      se.halfSpread = "Half-spread must be a finite value ≥ 0.";
    }
    if (s.kind === "INVENTORY_SKEW") {
      if (!isFinite(s.kappa)) se.kappa = "κ must be a finite number.";
      if (!isFinite(s.sMax) || s.sMax < 0) se.sMax = "Strategy sMax must be a finite value ≥ 0.";
    }
    if (Object.keys(se).length > 0) errors.strategies[i] = se;
  });

  const g = config.guardrails;
  if (g === null) {
    errors.guardrails.hMin = "Guardrails are required when tiering is enabled.";
  } else {
    if (!isFinite(g.hMin) || g.hMin < 0) errors.guardrails.hMin = "hMin must be a finite value ≥ 0.";
    if (!isFinite(g.hMax)) errors.guardrails.hMax = "hMax must be a finite value.";
    if (isFinite(g.hMin) && isFinite(g.hMax) && g.hMax < g.hMin) {
      errors.guardrails.hMax = "hMax must be ≥ hMin.";
    }
    if (!isFinite(g.sMax) || g.sMax < 0) errors.guardrails.sMax = "sMax must be a finite value ≥ 0.";
    if (!isFinite(g.spreadFloor) || g.spreadFloor <= 0) {
      errors.guardrails.spreadFloor = "spreadFloor must be a finite value > 0.";
    }
  }

  return errors;
}

/** Whether a {@link TieringErrors} carries any error (⇒ the config is invalid). */
export function hasTieringErrors(errors: TieringErrors): boolean {
  return (
    errors.form !== undefined ||
    Object.keys(errors.strategies).length > 0 ||
    Object.keys(errors.guardrails).length > 0
  );
}
