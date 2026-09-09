/**
 * Pure helpers for the outbound price-tiering config (FI-TIERING phase 3): the
 * option vocabularies + human labels the editor renders, sensible defaults for a
 * freshly enabled config, and the client-side validator that MIRRORS the server's
 * `celnet-tiering` guardrail invariants (see `docs/fixed-income/FI-TIERING-RESEARCH.md` §4/§5).
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

/** The strategy kinds the editor offers in the "add strategy" menu. */
export const TIERING_STRATEGY_KINDS: readonly TieringStrategyKind[] = [
  "FLAT_MARKUP",
  "INVENTORY_SKEW",
  "SCALED_SMOOTHED_SPREAD",
];

/** Human labels for each strategy kind. */
export const TIERING_STRATEGY_KIND_LABEL: Record<TieringStrategyKind, string> = {
  FLAT_MARKUP: "Flat markup",
  INVENTORY_SKEW: "Inventory skew",
  SCALED_SMOOTHED_SPREAD: "Scaled Smoothed Spread",
};

/** A one-line description of what each strategy does (rendered under its row). */
export const TIERING_STRATEGY_KIND_HINT: Record<TieringStrategyKind, string> = {
  FLAT_MARKUP: "Constant symmetric half-spread H around mid — no skew.",
  INVENTORY_SKEW: "Base half-spread H plus clamp(κ·inventory, ±sMax) skew.",
  SCALED_SMOOTHED_SPREAD:
    "Damps spread volatility: smooths the observed spread (EWMA weight w), then sets an absolute output spread O = min(m, c·(1 + f·D/e)) around mid. Use INSTEAD OF Flat markup.",
};

/**
 * The base documentation location the per-strategy "?" links point at — the
 * research corpus section authored alongside this feature. Each strategy's
 * {@link TieringStrategyMeta.docHref} deep-links to its own "how to use" heading.
 */
const TIERING_DOCS_BASE =
  "https://github.com/soarsa/celnet/blob/main/docs/fixed-income/FI-TIERING-RESEARCH.md";

/** User-facing "how to use it" metadata surfaced per strategy in the editor. */
export interface TieringStrategyMeta {
  /** The strategy's display title. */
  title: string;
  /** A one-sentence statement of what it is for. */
  purpose: string;
  /** A deep link to the strategy's "how to use" documentation section. */
  docHref: string;
}

/**
 * Per-strategy documentation registry: the title, a one-line purpose, and a link
 * to the "how to use" section of `docs/fixed-income/FI-TIERING-RESEARCH.md`. The editor renders
 * a help affordance per strategy from this table so a user can learn each one.
 */
export const TIERING_STRATEGY_META: Record<TieringStrategyKind, TieringStrategyMeta> = {
  FLAT_MARKUP: {
    title: "Flat markup",
    purpose: "A constant symmetric half-spread around mid — the simplest tier, no skew.",
    docHref: `${TIERING_DOCS_BASE}#flat-markup-how-to-use`,
  },
  INVENTORY_SKEW: {
    title: "Inventory skew",
    purpose:
      "A base half-spread plus a skew linear in signed inventory (clamped) to lean the book toward shedding risk.",
    docHref: `${TIERING_DOCS_BASE}#inventory-skew-how-to-use`,
  },
  SCALED_SMOOTHED_SPREAD: {
    title: "Scaled Smoothed Spread",
    purpose:
      "Damps spread volatility by smoothing the observed spread and scaling an absolute output spread from its divergence to an expected level.",
    docHref: `${TIERING_DOCS_BASE}#scaled-smoothed-spread-scale_smooth`,
  },
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

/**
 * A fresh strategy of `kind` with sensible starting magnitudes. FLAT_MARKUP /
 * INVENTORY_SKEW magnitudes are in the config unit; SCALED_SMOOTHED_SPREAD spread
 * params are ABSOLUTE price offsets — the worked-example ratios (e:d:c:m = 0.4:0.2:1:4,
 * f = 1.2) scaled into price points that sit comfortably inside the default
 * guardrails (core 0.2 above the 0.01 floor, max-output 0.8 below the 1.0 h_max·2).
 * Every field is always present (the wire emits them all); a `kind` ignores the
 * fields it does not use.
 */
export function defaultTieringStrategy(kind: TieringStrategyKind): TieringStrategy {
  // Ignored fields default to 0 so FLAT_MARKUP / INVENTORY_SKEW encode byte-identically
  // to the server's `TieringStrategyDesc::default()` (no round-trip drift on the fields
  // a kind does not use). SCALED_SMOOTHED_SPREAD overrides its own params below.
  const base: TieringStrategy = {
    kind,
    halfSpread: 25,
    kappa: 0,
    sMax: 0,
    smoothingWeight: 0,
    expectedSpread: 0,
    maxDivergence: 0,
    coreSpread: 0,
    maxOutputSpread: 0,
    spreadScaleFactor: 0,
  };
  if (kind === "INVENTORY_SKEW") {
    return { ...base, kappa: 0.5, sMax: 0.5 };
  }
  if (kind === "SCALED_SMOOTHED_SPREAD") {
    return {
      ...base,
      halfSpread: 0,
      smoothingWeight: 0.5,
      expectedSpread: 0.08,
      maxDivergence: 0.04,
      coreSpread: 0.2,
      maxOutputSpread: 0.8,
      spreadScaleFactor: 1.2,
    };
  }
  return base;
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
  smoothingWeight?: string;
  expectedSpread?: string;
  maxDivergence?: string;
  coreSpread?: string;
  maxOutputSpread?: string;
  spreadScaleFactor?: string;
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
    // Half-spread is the FLAT_MARKUP / INVENTORY_SKEW knob; SCALED_SMOOTHED_SPREAD
    // sets the spread from its own params and ignores half-spread.
    if (s.kind !== "SCALED_SMOOTHED_SPREAD" && (!isFinite(s.halfSpread) || s.halfSpread < 0)) {
      se.halfSpread = "Half-spread must be a finite value ≥ 0.";
    }
    if (s.kind === "INVENTORY_SKEW") {
      if (!isFinite(s.kappa)) se.kappa = "κ must be a finite number.";
      if (!isFinite(s.sMax) || s.sMax < 0) se.sMax = "Strategy sMax must be a finite value ≥ 0.";
    }
    if (s.kind === "SCALED_SMOOTHED_SPREAD") {
      // Mirrors the server's celnet-tiering config invariants for this strategy.
      if (!isFinite(s.smoothingWeight) || s.smoothingWeight <= 0 || s.smoothingWeight > 1) {
        se.smoothingWeight = "Smoothing weight w must be in (0, 1].";
      }
      if (!isFinite(s.expectedSpread) || s.expectedSpread <= 0) {
        se.expectedSpread = "Expected spread e must be a finite value > 0.";
      }
      if (!isFinite(s.maxDivergence) || s.maxDivergence < 0) {
        se.maxDivergence = "Max divergence d must be a finite value ≥ 0.";
      }
      if (!isFinite(s.coreSpread) || s.coreSpread < 0) {
        se.coreSpread = "Core spread c must be a finite value ≥ 0.";
      }
      if (!isFinite(s.maxOutputSpread) || s.maxOutputSpread < 0) {
        se.maxOutputSpread = "Max output spread m must be a finite value ≥ 0.";
      } else if (isFinite(s.coreSpread) && s.maxOutputSpread < s.coreSpread) {
        se.maxOutputSpread = "Max output spread m must be ≥ core spread c.";
      }
      if (!isFinite(s.spreadScaleFactor) || s.spreadScaleFactor < 0) {
        se.spreadScaleFactor = "Spread scale factor f must be a finite value ≥ 0.";
      }
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

// --- indicative outbound preview (the worked example, in-editor) --------------

/** A two-way bid/offer pair the preview surfaces. */
export interface PreviewTwoWay {
  bid: number;
  offer: number;
}

/**
 * The canonical worked-example raw composite: LP bid 99.50 / offer 99.60 (mid
 * 99.55, market spread 0.10). Flat ±25 price-bps tiering turns it into 99.30 /
 * 99.80 — the reference example in `docs/fixed-income/FI-TIERING-RESEARCH.md` §1/§9.
 */
export const TIERING_PREVIEW_RAW: PreviewTwoWay = { bid: 99.5, offer: 99.6 };

/** The sample signed inventory the INVENTORY_SKEW lean is computed against. */
const PREVIEW_INVENTORY = 1;

/**
 * Convert a spread magnitude expressed in `unit` to absolute PRICE POINTS around
 * `mid`. PRICE_BPS: 1 bp = 0.01 points (25 → 0.25). PRICE_POINTS: as-is. PERCENT:
 * `mid · v/100`. YIELD_BPS is DV01-dependent on the server; the in-editor preview
 * treats it like price bps as an INDICATIVE view (the outbound label flags this).
 */
export function spreadUnitToPoints(value: number, unit: TieringSpreadUnit, mid: number): number {
  switch (unit) {
    case "PRICE_BPS":
    case "YIELD_BPS":
      return value / 100;
    case "PRICE_POINTS":
      return value;
    case "PERCENT":
      return (mid * value) / 100;
    default:
      return value;
  }
}

function clampNum(x: number, lo: number, hi: number): number {
  return Math.max(lo, Math.min(hi, x));
}

/**
 * Compute the INDICATIVE outbound two-way a tiering config produces from a raw
 * composite (defaults to {@link TIERING_PREVIEW_RAW}). This mirrors the server's
 * pipeline enough to make the editor legible — it is NOT the authoritative price:
 *
 *  - SCALED_SMOOTHED_SPREAD present ⇒ it is the spread SOURCE: the output spread
 *    `O = min(m, c·(1 + P))` from the divergence of the observed raw spread
 *    (single-shot, S = R with no history); the two-way is symmetric `M ± O/2`.
 *  - otherwise ⇒ half-spread `h = Σ FLAT/INVENTORY halfSpread` (unit-converted).
 *  - INVENTORY_SKEW adds a lean `s = Σ clamp(κ·q, ±sMax)` (unit-converted), shifting
 *    the whole two-way: `bid = M − h − s, offer = M + h − s`.
 *  - guardrails clamp `h ∈ [hMin, hMax]`, enforce `spread_floor`, and cap `|s| ≤ sMax`.
 *
 * A disabled config (`null`) returns the raw composite unchanged.
 */
export function outboundTwoWayPreview(
  config: TieringConfig | null,
  raw: PreviewTwoWay = TIERING_PREVIEW_RAW,
): PreviewTwoWay {
  const mid = (raw.bid + raw.offer) / 2;
  const rawSpread = raw.offer - raw.bid;
  if (config === null) return { ...raw };

  const scaled = config.strategies.find((s) => s.kind === "SCALED_SMOOTHED_SPREAD");
  let half: number;
  if (scaled) {
    // Divergence of the observed raw spread from the expected level (S = R, one-shot).
    const d = Math.abs(rawSpread - scaled.expectedSpread);
    const p =
      d <= scaled.maxDivergence || scaled.expectedSpread === 0
        ? 0
        : (scaled.spreadScaleFactor * d) / scaled.expectedSpread;
    const output = Math.min(scaled.maxOutputSpread, scaled.coreSpread * (1 + p));
    half = output / 2;
  } else {
    half = config.strategies.reduce(
      (sum, s) =>
        s.kind === "FLAT_MARKUP" || s.kind === "INVENTORY_SKEW"
          ? sum + spreadUnitToPoints(s.halfSpread, config.unit, mid)
          : sum,
      0,
    );
  }

  let skew = config.strategies.reduce((sum, s) => {
    if (s.kind !== "INVENTORY_SKEW") return sum;
    const kappaPts = spreadUnitToPoints(s.kappa * PREVIEW_INVENTORY, config.unit, mid);
    const sMaxPts = spreadUnitToPoints(s.sMax, config.unit, mid);
    return sum + clampNum(kappaPts, -sMaxPts, sMaxPts);
  }, 0);

  const g = config.guardrails;
  if (g !== null) {
    half = clampNum(half, g.hMin, g.hMax);
    if (2 * half < g.spreadFloor) half = g.spreadFloor / 2;
    skew = clampNum(skew, -g.sMax, g.sMax);
  }

  return { bid: mid - half - skew, offer: mid + half - skew };
}
