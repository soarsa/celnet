/**
 * The ProductSpec registry contract (GW2). One structurable product family =
 * one {@link ProductSpec}. Adding a product is adding a registry entry — no edit
 * to the ticket shell — which is what lets the 19-structure catalogue (and the
 * multi-asset future) scale without the former 3606-line `TicketWorkspace`
 * monolith. The wire instrument is built by `toInstrument`, which MUST reproduce
 * the legacy `buildInstrument` output byte-for-byte (gated by the registry
 * round-trip + the existing product tests).
 */
import type { ReactElement } from "react";
import type { Instrument, PricingModel, Product, Tenor } from "../data/contract";
import type { BrokenDate, SettlementStyle, Underlying } from "../data/contract";

/**
 * Asset class of a structurable product. FX is the origin class; the cross-asset
 * integration wave adds METAL / EQUITY / COMMODITY / CRYPTO as registry DATA (new
 * specs over the W1 `Underlying` oneof seam), not a shell rewrite — the registry
 * is the seam that makes that a data change. Each non-FX class books through the
 * same `Instrument.product` oneof, carrying its asset-class identity on
 * `Instrument.underlying` (and, for CRYPTO, `Instrument.settlementStyle`).
 */
export type AssetClass = "FX" | "METAL" | "EQUITY" | "COMMODITY" | "CRYPTO";

/** Gallery grouping for the structure picker (replaces the flat 19-item `<select>`). */
export type ProductGroup =
  | "Vanilla & strategies"
  | "Linear (forwards & swaps)"
  | "Barriers & digitals"
  | "Volatility"
  | "Path-dependent"
  | "Structured"
  | "Cross-asset (equity / commodity / crypto)";

/** The canonical group order in the structure gallery. */
export const PRODUCT_GROUP_ORDER: readonly ProductGroup[] = [
  "Vanilla & strategies",
  "Linear (forwards & swaps)",
  "Barriers & digitals",
  "Volatility",
  "Path-dependent",
  "Structured",
  "Cross-asset (equity / commodity / crypto)",
];

/**
 * The market / contract context a {@link ProductSpec} needs to build its wire
 * instrument and render its input block. Supplied by the ticket shell from the
 * live pair / tenor / marked surface.
 */
export interface ProductBuildCtx {
  /** The currency pair (base/quote 3-letter codes). */
  pair: { base: string; quote: string };
  /**
   * The active underlier's asset class (FX for a plain FX pair). Drives the
   * class-aware discovery layer (the capability matrix in `capability.ts`), the
   * Greeks / convention labels, and the risk shock axes — read ONLY by those
   * class-aware consumers. Absent ⇒ treat as FX, so every existing ctx and the
   * instrument a spec builds stay byte-identical (the economics ride `pair` /
   * `underlying`, never this field).
   */
  assetClass?: AssetClass;
  /**
   * The ACTIVE non-FX underlier identity, when one is overlaid (the universe-leaf
   * cross-asset selection). The asset-class-AGNOSTIC arms — the perpetual and the
   * listed-future-option, which price END-TO-END on equity/commodity/crypto via the
   * server's cost-of-carry seam — read it to carry the cross-asset wire keys
   * (`Instrument.underlying` + `settlement_style`) onto their built instrument,
   * reusing the same `Underlying` the cross-asset vanilla seeds from (no parallel
   * mechanism). Absent ⇒ FX (the resting class): every spec builds its FX/metal
   * instrument byte-identically, the economics ride `pair` exactly as before. The
   * FX-native exotics never read it — they are FX/metal-only and gated out of the
   * cross-asset classes by the capability matrix.
   */
  underlier?: { underlying: Underlying; settlementStyle: SettlementStyle };
  /** The selected tenor (stamped onto the instrument; `expiryYears` stays authoritative). */
  tenor: Tenor;
  /** The tenor as a year fraction (the pricing-authoritative expiry). */
  tenorYears: number;
  /** Notional in millions of base. */
  notionalMm: number;
  /** The trader's booking-model selection (DEFAULT unless overridden). */
  pricingModel: PricingModel;
  /** The ATM-forward level for (pair, tenor); strikes that default to ATMF read this. */
  atmForward: number;
  /** Spot for the pair (touch barriers read this). */
  spot: number;
  /** Quote-pip decimals, for number-field precision / formatting. */
  pipDecimals: number;
  /** Today (UTC), for date-anchored products (forward-start / American schedules). */
  today: BrokenDate;
}

/** Props every product {@link ProductSpec.InputBlock} receives. */
export interface InputBlockProps<I> {
  /** The current inputs for this family. */
  value: I;
  /** Replace the inputs (the shell owns the per-family state). */
  onChange: (next: I) => void;
  /** Live market / contract context. */
  ctx: ProductBuildCtx;
}

/** One structurable product family — its identity, metadata, defaults, builder and UI. */
export interface ProductSpec<I> {
  /** Stable structure id (matches the ticket `Structure` union + analytics catalogue). */
  id: string;
  /** Trader-facing name. */
  label: string;
  /** Gallery group. */
  group: ProductGroup;
  /** Asset class (the spec's origin/identity class). */
  assetClass: AssetClass;
  /**
   * The asset classes this spec can STRUCTURE a priceable instrument for. Absent ⇒
   * the FX-native default [FX, METAL] (the FX engine also prices a metal pair). The
   * cross-asset vanilla gateway + the asset-class-agnostic arms (perpetual /
   * listed-future-option) widen this. The discovery gallery reads it (via
   * `capability.galleryCardStates`) to show only the products a trader can actually
   * build on the active underlier, dimming the rest with the honest reason.
   */
  applicableClasses?: readonly AssetClass[];
  /** One-line gallery description. */
  summary: string;
  /** Extra search keywords for the gallery (method synonyms, aliases). */
  keywords: readonly string[];
  /** The proto product-oneof arm this family books as (drives {@link ProductSpec.allowedModels}). */
  kind: Product["kind"];
  /** Default inputs at first render. */
  defaults: I;
  /** The booking models valid for this family (from `bookingModelsFor(kind)`). */
  allowedModels: readonly PricingModel[];
  /**
   * Declared ONLY by a family with no expiry/tenor dimension (the perpetual
   * option — the one tenorless, expiryless product on the contract). The ticket
   * shell disables the expiry controls and shows the `reason` in their place,
   * and the family's `toInstrument` MUST encode the contract's canonical
   * no-expiry shape (`expiryYears = 0` exactly, no `tenor`). Absent ⇒ an
   * ordinary dated product (the expiry controls behave as ever).
   */
  noExpiry?: { reason: string };
  /**
   * Validate the inputs against the family's structure laws (e.g. the strategy
   * leg-ladder templates: a risk reversal pairs a call against a put). Returns
   * display-ready violation messages; non-empty gates the shell's Request quote
   * (the wire would otherwise carry a structure that belies its declared
   * template). The family's InputBlock renders the same messages inline. Absent
   * ⇒ every input state the block can produce is lawful.
   */
  validate?: (inputs: I, ctx: ProductBuildCtx) => readonly string[];
  /**
   * Build the wire {@link Instrument} from the trader inputs + market context.
   * MUST match the legacy `buildInstrument` output byte-for-byte. Use
   * {@link withTenorAndModel} to stamp the tenor + booking model identically.
   * Total for EVERY committed input state (the shell builds on every render),
   * including states `validate` flags — law violations gate pricing, not build.
   */
  toInstrument: (inputs: I, ctx: ProductBuildCtx) => Instrument;
  /** The input block UI for this family. */
  InputBlock: (props: InputBlockProps<I>) => ReactElement;
}

/**
 * Type-erased {@link ProductSpec} for heterogeneous registry storage/iteration.
 * The per-family `I` is recovered at the single shell boundary that owns that
 * family's input state. (A registry of differently-typed specs is the one place
 * an erased element type is sound — every consumer pairs a spec with its own
 * matching state.)
 */
// eslint-disable-next-line @typescript-eslint/no-explicit-any
export type AnyProductSpec = ProductSpec<any>;

/** Identity helper that preserves a spec's `I` at definition while typing the field. */
export function defineProduct<I>(spec: ProductSpec<I>): ProductSpec<I> {
  return spec;
}

/**
 * Stamp the trader tenor + booking model onto a freshly-built base instrument,
 * reproducing the legacy `buildInstrument` tail exactly: DEFAULT is
 * presence-omitted on the wire (so an analytic instrument is byte-identical to
 * the legacy frame); a `lockedModel` (e.g. the window barrier's LOCAL_STOCH_VOL,
 * which has no closed form) overrides the trader selection.
 */
export function withTenorAndModel(
  base: Instrument,
  ctx: ProductBuildCtx,
  lockedModel?: PricingModel,
): Instrument {
  return withModel({ ...base, tenor: ctx.tenor }, ctx, lockedModel);
}

/**
 * Stamp ONLY the booking model (the {@link withTenorAndModel} tail without the
 * tenor stamp) — for the one tenorless family (the perpetual), whose instrument
 * must carry NO tenor. DEFAULT stays presence-omitted on the wire exactly as in
 * {@link withTenorAndModel}.
 */
export function withModel(
  base: Instrument,
  ctx: ProductBuildCtx,
  lockedModel?: PricingModel,
): Instrument {
  const resolved: PricingModel = lockedModel ?? ctx.pricingModel;
  const instrument: Instrument = { ...base };
  if (resolved !== "DEFAULT") instrument.pricingModel = resolved;
  return instrument;
}
