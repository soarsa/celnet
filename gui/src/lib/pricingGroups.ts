/**
 * Pure helpers for the FI Pricing-Groups pipeline builder (docs/FI-PRICING-GROUPS-
 * DESIGN.md §6 + §8.5): the feature-library vocabularies + human labels the palette
 * renders, sensible defaults for each freshly dropped feature, the client-side
 * validator (which MIRRORS the server's guardrail invariants and reuses the shipped
 * tiering validator verbatim for a TIERING feature), the pure add/remove/reorder
 * pipeline ops the builder's reducer composes, and an INDICATIVE two-way price
 * WATERFALL preview so the trader sees the running bid/offer AFTER each feature.
 *
 * No React, no transport — so the workspace and the vitest suite share ONE source
 * of truth. The server still enforces every invariant and computes the AUTHORITATIVE
 * price; this preview is a client-side indication (treats every magnitude as price
 * points) to make the pipeline legible while the trader builds it.
 */

import type {
  AxeSide,
  FeatureKind,
  FeaturePipeline,
  FeatureSpec,
  LastLookMode,
  PricingGroup,
  PricingSourceMode,
  TieringGuardrails,
} from "../data/contract";
import {
  DEFAULT_TIERING_GUARDRAILS,
  defaultTieringConfig,
  hasTieringErrors,
  validateTiering,
  type TieringErrors,
} from "./tiering";

// --- feature-library vocabularies + labels (the palette) ----------------------

/** The feature kinds, in wire (enum) order — the palette's chip order. */
export const FEATURE_KINDS: readonly FeatureKind[] = [
  "MID_SHIFT",
  "TIERING",
  "AXE",
  "POSITION",
  "PANIC_SKEW",
];

/** Human labels for each feature kind (the palette chip + card heading text). */
export const FEATURE_KIND_LABEL: Record<FeatureKind, string> = {
  MID_SHIFT: "MID SHIFT",
  TIERING: "TIERING",
  AXE: "AXE",
  POSITION: "POSITION",
  PANIC_SKEW: "PANIC/SKEW",
};

/** A one-line description of what each feature does (rendered under its chip/card). */
export const FEATURE_KIND_HINT: Record<FeatureKind, string> = {
  MID_SHIFT: "Shift mid by a signed offset, or override it with an absolute reference price.",
  TIERING: "Widen the two-way around mid using the shared tiering strategies + guardrails.",
  AXE: "Skew mid toward the side the desk wants to trade to attract that flow.",
  POSITION: "Lean the price against signed inventory (κ per unit, clamped at sMax) to shed risk.",
  PANIC_SKEW: "An emergency mid overlay applied only while the panic flag is triggered.",
};

/** The AXE sides, in wire (enum) order. */
export const AXE_SIDES: readonly AxeSide[] = ["BUY", "SELL"];

/** Human labels for each AXE side. */
export const AXE_SIDE_LABEL: Record<AxeSide, string> = {
  BUY: "Buy (lift mid to attract sellers)",
  SELL: "Sell (drop mid to attract buyers)",
};

// --- pricing-source policy (how the raw rates/bond price is sourced) ----------

/** The pricing-source modes, in wire (enum) order — the selector's option order. */
export const PRICING_SOURCE_MODES: readonly PricingSourceMode[] = [0, 1, 2, 3, 4];

/** Trader-language label for each pricing-source mode (the selector option text). */
export const PRICING_SOURCE_MODE_LABEL: Record<PricingSourceMode, string> = {
  0: "Composite-first (book, else curve)",
  1: "Curve only",
  2: "Product split — bonds off book, OIS off curve",
  3: "Curve-anchored + book skew",
  4: "Composite only — decline when the book cannot source it",
};

/** A one-line explanation of each mode (rendered as helper text under the selector). */
export const PRICING_SOURCE_MODE_HINT: Record<PricingSourceMode, string> = {
  0: "Price off the aggregated book when it is fed; fall back to the bootstrapped curve otherwise.",
  1: "Always price off the bootstrapped curve, ignoring the aggregated book.",
  2: "Bonds price off the aggregated book; OIS prices off the curve.",
  3: "Curve backbone with the mid pulled toward the composite by the book-skew weight.",
  4: "Quote ONLY what the aggregated book can source, and decline otherwise — never invent a price off the curve. Choose this when the desk quotes as agent of real consolidated liquidity rather than warehousing risk on an internal mark.",
};

/** The server's default book-skew weight (shown when the group has no stored value). */
export const DEFAULT_BOOK_SKEW_WEIGHT = 0.5;

/** The mode (`3`) for which {@link PricingGroup.bookSkewWeight} is meaningful. */
export const CURVE_ANCHORED_BOOK_SKEW_MODE: PricingSourceMode = 3;

// --- last-look policy (how a streamed-quote lift is honored on a market move) --

/** The last-look modes, in wire (enum) order — the selector's option order. */
export const LAST_LOOK_MODES: readonly LastLookMode[] = [0, 1];

/** Trader-language label for each last-look mode (the selector option text). */
export const LAST_LOOK_MODE_LABEL: Record<LastLookMode, string> = {
  0: "Sync — desk keeps the full favorable move",
  1: "Async — client gets a % of the favorable move back",
};

/** A one-line explanation of each mode (rendered as helper text under the selector). */
export const LAST_LOOK_MODE_HINT: Record<LastLookMode, string> = {
  0: "The client is filled at exactly the price they requested; the desk keeps the entire favorable move.",
  1: "The client gets price improvement — a share of the favorable move is passed back to them; the desk keeps the rest.",
};

/** The server's default adverse-move tolerance in bps (shown when the group has no stored value). */
export const DEFAULT_LAST_LOOK_TOLERANCE_BPS = 1.0;

/** The server's default async giveback % (shown when the group has no stored value). */
export const DEFAULT_ASYNC_GIVEBACK_PCT = 50;

/** The mode (`1`) for which {@link PricingGroup.asyncGivebackPct} is meaningful. */
export const ASYNC_LAST_LOOK_MODE: LastLookMode = 1;

// --- defaults ----------------------------------------------------------------

/**
 * A fresh feature of `kind` with EVERY field present (the wire emits them all, so a
 * fully-populated spec round-trips byte-stably) and sensible starting magnitudes for
 * the fields the kind actually uses. Fields a kind ignores stay at the all-zero base
 * (unit PRICE_POINTS, reference null, tiering null, axeSide BUY, triggered false).
 */
export function defaultFeatureSpec(kind: FeatureKind): FeatureSpec {
  const base: FeatureSpec = {
    kind,
    unit: "PRICE_POINTS",
    shift: 0,
    reference: null,
    tiering: null,
    axeSide: "BUY",
    magnitude: 0,
    kappa: 0,
    sMax: 0,
    skew: 0,
    triggered: false,
  };
  switch (kind) {
    case "MID_SHIFT":
      return { ...base, shift: 0.05 };
    case "TIERING":
      return { ...base, tiering: defaultTieringConfig() };
    case "AXE":
      return { ...base, axeSide: "BUY", magnitude: 0.02 };
    case "POSITION":
      return { ...base, kappa: 0.5, sMax: 0.05 };
    case "PANIC_SKEW":
      return { ...base, skew: 0.02, triggered: false };
    default:
      return base;
  }
}

/** A fresh empty pipeline (no features) carrying the default price-space guardrails. */
export function defaultPipeline(): FeaturePipeline {
  return { features: [], guardrails: { ...DEFAULT_TIERING_GUARDRAILS } };
}

// --- pure pipeline ops (the builder reducer + drag-drop compose these) --------

/** Insert `feature` at `index` (clamped into `[0, len]`), returning a NEW array. */
export function insertFeatureAt(
  features: readonly FeatureSpec[],
  feature: FeatureSpec,
  index: number,
): FeatureSpec[] {
  const at = Math.max(0, Math.min(features.length, index));
  return [...features.slice(0, at), feature, ...features.slice(at)];
}

/** Remove the feature at `index` (out-of-range ⇒ unchanged copy), returning a NEW array. */
export function removeFeatureAt(features: readonly FeatureSpec[], index: number): FeatureSpec[] {
  if (index < 0 || index >= features.length) return [...features];
  return features.filter((_, i) => i !== index);
}

/** Move the feature at `from` to `to` (both clamped), returning a NEW reordered array. */
export function moveFeature(
  features: readonly FeatureSpec[],
  from: number,
  to: number,
): FeatureSpec[] {
  if (from < 0 || from >= features.length) return [...features];
  const dest = Math.max(0, Math.min(features.length - 1, to));
  if (dest === from) return [...features];
  const next = [...features];
  const [moved] = next.splice(from, 1);
  next.splice(dest, 0, moved as FeatureSpec);
  return next;
}

/** Patch the feature at `index` with `next`, returning a NEW array (immutable). */
export function updateFeatureAt(
  features: readonly FeatureSpec[],
  index: number,
  next: Partial<FeatureSpec>,
): FeatureSpec[] {
  return features.map((f, i) => (i === index ? { ...f, ...next } : f));
}

// --- validation (mirrors the server's guardrail invariants) ------------------

/** Per-field errors for one feature card (absent key ⇒ that field is valid). */
export interface FeatureErrors {
  shift?: string;
  reference?: string;
  magnitude?: string;
  kappa?: string;
  sMax?: string;
  skew?: string;
  /** The reused tiering validator's errors (TIERING feature only). */
  tiering?: TieringErrors;
}

/** Guardrail-field errors for a pipeline (all-empty ⇒ valid). */
export interface GuardrailErrors {
  hMin?: string;
  hMax?: string;
  sMax?: string;
  spreadFloor?: string;
}

/** The structured error set for a whole pipeline (all-empty ⇒ valid). */
export interface PipelineErrors {
  /** Per-feature errors, keyed by index. */
  features: Record<number, FeatureErrors>;
  /** Pipeline guardrail-field errors. */
  guardrails: GuardrailErrors;
}

/** Field-level errors for the group's structural fields (name). */
export interface PricingGroupErrors {
  name?: string;
}

function finite(x: number): boolean {
  return Number.isFinite(x);
}

/** Validate one feature spec against the invariants its `kind` uses. */
function validateFeature(f: FeatureSpec): FeatureErrors {
  const fe: FeatureErrors = {};
  switch (f.kind) {
    case "MID_SHIFT":
      if (!finite(f.shift)) fe.shift = "Shift must be a finite value.";
      if (f.reference !== null && !finite(f.reference)) {
        fe.reference = "Reference price must be a finite value.";
      }
      break;
    case "TIERING": {
      if (f.tiering === null) {
        fe.tiering = { form: "A TIERING feature must carry a tiering config.", strategies: {}, guardrails: {} };
      } else {
        const te = validateTiering(f.tiering);
        if (hasTieringErrors(te)) fe.tiering = te;
      }
      break;
    }
    case "AXE":
      if (!finite(f.magnitude) || f.magnitude < 0) {
        fe.magnitude = "Magnitude must be a finite value ≥ 0.";
      }
      break;
    case "POSITION":
      if (!finite(f.kappa)) fe.kappa = "κ must be a finite number.";
      if (!finite(f.sMax) || f.sMax < 0) fe.sMax = "sMax must be a finite value ≥ 0.";
      break;
    case "PANIC_SKEW":
      if (!finite(f.skew)) fe.skew = "Skew must be a finite value.";
      break;
    default:
      break;
  }
  return fe;
}

/** Validate the pipeline's optional guardrail block (null ⇒ no guardrails, valid). */
function validateGuardrails(g: TieringGuardrails | null): GuardrailErrors {
  const ge: GuardrailErrors = {};
  if (g === null) return ge;
  if (!finite(g.hMin) || g.hMin < 0) ge.hMin = "hMin must be a finite value ≥ 0.";
  if (!finite(g.hMax)) ge.hMax = "hMax must be a finite value.";
  if (finite(g.hMin) && finite(g.hMax) && g.hMax < g.hMin) ge.hMax = "hMax must be ≥ hMin.";
  if (!finite(g.sMax) || g.sMax < 0) ge.sMax = "sMax must be a finite value ≥ 0.";
  if (!finite(g.spreadFloor) || g.spreadFloor <= 0) {
    ge.spreadFloor = "spreadFloor must be a finite value > 0.";
  }
  return ge;
}

/**
 * Validate a whole pipeline: each feature against its kind's invariants (a TIERING
 * feature reuses {@link validateTiering} verbatim) plus the optional guardrail block.
 * Returns a structured set; {@link hasPipelineErrors} collapses it to a boolean.
 */
export function validatePipeline(pipeline: FeaturePipeline): PipelineErrors {
  const errors: PipelineErrors = { features: {}, guardrails: {} };
  pipeline.features.forEach((f, i) => {
    const fe = validateFeature(f);
    if (Object.keys(fe).length > 0) errors.features[i] = fe;
  });
  errors.guardrails = validateGuardrails(pipeline.guardrails);
  return errors;
}

/** Whether a {@link PipelineErrors} carries any error (⇒ the pipeline is invalid). */
export function hasPipelineErrors(errors: PipelineErrors): boolean {
  return (
    Object.keys(errors.features).length > 0 || Object.keys(errors.guardrails).length > 0
  );
}

/** Validate a pricing group's structural fields (a non-empty, trimmed name). */
export function validatePricingGroup(group: PricingGroup): PricingGroupErrors {
  const errors: PricingGroupErrors = {};
  if (group.name.trim().length === 0) errors.name = "Group name is required.";
  return errors;
}

/** Whether a {@link PricingGroupErrors} carries any error. */
export function hasPricingGroupErrors(errors: PricingGroupErrors): boolean {
  return errors.name !== undefined;
}

// --- two-way preview WATERFALL (indicative) ----------------------------------

/** A running two-way price (a bid/offer pair) the preview waterfall carries. */
export interface TwoWay {
  bid: number;
  offer: number;
}

/** The sample raw two-way fed through the active pipeline for the preview. */
export const SAMPLE_RAW: TwoWay = { bid: 99.5, offer: 99.6 };

/** The sample signed net inventory the POSITION feature skews against. */
export const SAMPLE_INVENTORY = 1;

/** clamp `x` into `[lo, hi]` (assumes `lo <= hi`). */
function clamp(x: number, lo: number, hi: number): number {
  return Math.max(lo, Math.min(hi, x));
}

const midOf = (tw: TwoWay): number => (tw.bid + tw.offer) / 2;
const halfOf = (tw: TwoWay): number => (tw.offer - tw.bid) / 2;

/** Rebuild a two-way from a mid + half-spread. */
function twoWay(mid: number, half: number): TwoWay {
  return { bid: mid - half, offer: mid + half };
}

/**
 * Apply ONE feature to a running two-way (indicative — every magnitude is treated
 * as price points). Returns a NEW two-way; the input is never mutated. Each kind's
 * transform mirrors the design's pipeline semantics (§6):
 *  - MID_SHIFT: mid ← `reference` when set, else mid + `shift`; half-spread kept.
 *  - TIERING: half-spread widened by the sum of the config's FLAT_MARKUP /
 *    INVENTORY_SKEW strategy `halfSpread` values, then clamped into the config's own
 *    guardrails `[hMin, hMax]` when present; mid kept.
 *  - AXE: mid leaned by ±`magnitude` toward `axeSide`; spread kept.
 *  - POSITION: mid reduced by `clamp(κ·inventory, ±sMax)` (long ⇒ lower to shed); spread kept.
 *  - PANIC_SKEW: mid + `skew` only while `triggered`; spread kept.
 */
export function applyFeature(tw: TwoWay, f: FeatureSpec): TwoWay {
  const mid = midOf(tw);
  const half = halfOf(tw);
  switch (f.kind) {
    case "MID_SHIFT": {
      const nextMid = f.reference !== null ? f.reference : mid + f.shift;
      return twoWay(nextMid, half);
    }
    case "TIERING": {
      if (f.tiering === null) return twoWay(mid, half);
      const widen = f.tiering.strategies.reduce(
        (sum, s) =>
          s.kind === "FLAT_MARKUP" || s.kind === "INVENTORY_SKEW" ? sum + s.halfSpread : sum,
        0,
      );
      let nextHalf = half + widen;
      const g = f.tiering.guardrails;
      if (g !== null) nextHalf = clamp(nextHalf, g.hMin, g.hMax);
      return twoWay(mid, nextHalf);
    }
    case "AXE": {
      const nextMid = f.axeSide === "BUY" ? mid + f.magnitude : mid - f.magnitude;
      return twoWay(nextMid, half);
    }
    case "POSITION": {
      const skew = clamp(f.kappa * SAMPLE_INVENTORY, -f.sMax, f.sMax);
      return twoWay(mid - skew, half);
    }
    case "PANIC_SKEW": {
      const nextMid = f.triggered ? mid + f.skew : mid;
      return twoWay(nextMid, half);
    }
    default:
      return twoWay(mid, half);
  }
}

/** Clamp a two-way's half-spread into `[hMin, hMax]` and enforce `spreadFloor`. */
function applyGuardrails(tw: TwoWay, g: TieringGuardrails): TwoWay {
  const mid = midOf(tw);
  let half = clamp(halfOf(tw), g.hMin, g.hMax);
  if (2 * half < g.spreadFloor) half = g.spreadFloor / 2;
  return twoWay(mid, half);
}

/**
 * Feed `raw` through the ordered `features` and return the WATERFALL: index 0 is the
 * raw two-way, then the running two-way AFTER each feature (so the result has length
 * `features.length + 1`). When `guardrails` is present it clamps the FINAL composed
 * two-way (the last element) — the raw entry always stays pristine.
 */
export function previewPipeline(
  raw: TwoWay,
  features: readonly FeatureSpec[],
  guardrails: TieringGuardrails | null,
): TwoWay[] {
  const steps: TwoWay[] = [{ ...raw }];
  let cur = raw;
  for (const f of features) {
    cur = applyFeature(cur, f);
    steps.push(cur);
  }
  if (guardrails !== null && features.length > 0) {
    steps[steps.length - 1] = applyGuardrails(steps[steps.length - 1] as TwoWay, guardrails);
  }
  return steps;
}
