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
import type { RatesCurveSet, RatesInstrument } from "../data/contract";

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
  | "Cross-asset (equity / commodity / crypto)"
  | "Fixed income (rates)";

/** The canonical group order in the structure gallery. */
export const PRODUCT_GROUP_ORDER: readonly ProductGroup[] = [
  "Vanilla & strategies",
  "Linear (forwards & swaps)",
  "Barriers & digitals",
  "Volatility",
  "Path-dependent",
  "Structured",
  "Cross-asset (equity / commodity / crypto)",
  // fe-fi-migration #3: the fixed-income (linear rates) family — the OIS priced
  // through the SAME ticket as FX/cross-asset, collapsing the standalone FI
  // pricing silo. Its specs are a distinct pricing family (see {@link RatesProductSpec}).
  "Fixed income (rates)",
];

/**
 * The pricing family a {@link ProductSpec}/{@link RatesProductSpec} belongs to.
 * `option` (the resting family, absent ⇒ this) builds a wire {@link Instrument}
 * the ticket prices via `requestQuote` (an FX/cross-asset premium two-way + the
 * 14-Greek set). `rates` builds an {@link OisInstrument} the ticket prices via
 * `priceRates` against a calibrated {@link RatesCurveSet} (PV / par rate / PV01 /
 * DV01 / key-rate ladder) — the fixed-income fold, one ticket for both.
 */
export type ProductFamily = "option" | "rates";

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
  /**
   * The pricing family. Absent (or `"option"`) ⇒ the FX/cross-asset options family
   * (this interface): the ticket builds {@link ProductSpec.toInstrument} and prices
   * it via `requestQuote`. The `"rates"` family is a distinct shape,
   * {@link RatesProductSpec}. This literal is the discriminant of {@link AnyProductSpec}.
   */
  family?: "option";
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
 * How the shared ticket labels + shapes a rates family's {@link RatesPricingResult}.
 * The default (absent ⇒ the OIS/swap view) shows PV, the par (fair fixed) rate, PV01,
 * DV01, and the key-rate DV01 ladder. The cash bond overrides this: its wire result
 * carries `pv = dirty price`, `parRate = yield to maturity`, `pv01 = dv01` (the yield
 * DV01), and an EMPTY ladder — so it hides the PV01 row + the ladder and relabels PV
 * and the par metric. The renderer only ever shows fields the wire actually carries
 * (no fabricated clean price / duration — those are not on `RatesPricingResult`).
 */
export interface RatesResultView {
  /** Label for the PV metric (default "PV"; bond "Dirty PV"). */
  pvLabel: string;
  /** Whether the PV metric carries the curve-currency unit (bond price: false). Default true. */
  pvHasCurrencyUnit?: boolean;
  /** Label for the par-rate metric (default "Par rate"; bond "Yield to maturity"). */
  parLabel: string;
  /** Show the analytic PV01 row (default true; bond false — PV01 ≡ DV01). */
  showPv01?: boolean;
  /** Show the key-rate DV01 ladder (default true; bond false — the wire ladder is empty). */
  showLadder?: boolean;
}

/**
 * A fixed-income (linear rates) product family — the OIS fold (fe-fi-migration #3),
 * generalised to the vanilla IRS, FRA and cash bond arms (fi-bond-ticket-gui).
 * Shares the discovery metadata + per-family input state seam of {@link ProductSpec},
 * but is priced through a DIFFERENT wire path: it builds a {@link RatesInstrument}
 * (one of the `ois` / `irs` / `fra` / `bond` arms) the ticket prices via
 * `CelnetTransport.priceRates` against a calibrated {@link RatesCurveSet} (PV / par or
 * yield / PV01 / DV01 / key-rate ladder) — NOT the option `Instrument`/`requestQuote`
 * two-way. It therefore has no `Instrument` product-oneof `kind`, no `allowedModels`
 * (no booking model), and no expiry/tenor shell dimension (the tenor/maturity is an
 * input its {@link ProductSpec.InputBlock} owns). The `"rates"` `family` literal
 * discriminates {@link AnyProductSpec}.
 */
export interface RatesProductSpec<I> {
  /** Stable structure id (matches the ticket `Structure` selection + analytics catalogue). */
  id: string;
  /** The fixed-income pricing family — the {@link AnyProductSpec} discriminant. */
  family: "rates";
  /** Trader-facing name. */
  label: string;
  /** Gallery group. */
  group: ProductGroup;
  /** One-line gallery description. */
  summary: string;
  /** Extra search keywords for the gallery (method synonyms, aliases). */
  keywords: readonly string[];
  /** Default inputs at first render. */
  defaults: I;
  /** The calibrated curve the family prices against (e.g. the default USD-SOFR curve). */
  curve: RatesCurveSet;
  /**
   * Validate the inputs against the family's laws (a whole-year tenor `>= 1`, a
   * positive notional). Returns display-ready violation messages; non-empty gates
   * the shell's price request. Absent ⇒ every input state the block can produce is lawful.
   */
  validate?: (inputs: I, ctx: ProductBuildCtx) => readonly string[];
  /**
   * Build the wire {@link RatesInstrument} (the priced oneof arm) from the trader
   * inputs. The ticket prices it via `priceRates(curve, instrument)`; the offline
   * in-app source and the live `price_rates` mirror compute the SAME real result off
   * the one contract.
   */
  toRatesInstrument: (inputs: I, ctx: ProductBuildCtx) => RatesInstrument;
  /**
   * How the shared ticket labels the priced result. Absent ⇒ the OIS/swap view
   * (PV / par rate / PV01 / DV01 / key-rate ladder). The bond overrides it.
   */
  resultView?: RatesResultView;
  /**
   * The verb on the price button ("Price OIS" / "Price swap" / "Price FRA" /
   * "Price bond"). Absent ⇒ a plain "Price".
   */
  priceActionLabel?: string;
  /** The empty-state hint shown before the first price. */
  emptyHint?: string;
  /**
   * Set the family's fixed rate to the just-priced par (breakeven) rate — the rates
   * analogue of the FX inline strike solve. Declared ONLY by families that carry a
   * fixed-rate input (OIS / IRS / FRA); absent ⇒ no "Set to par" action (the bond,
   * whose par metric is a yield with no fixed-rate input to pin). Returns the updated
   * inputs; `parRate` is the wire result's `parRate` (a decimal).
   */
  pinToPar?: (inputs: I, parRate: number) => I;
  /** The input block UI for this family. */
  InputBlock: (props: InputBlockProps<I>) => ReactElement;
}

/**
 * Type-erased product family for heterogeneous registry storage/iteration — the
 * discriminated union of the FX/cross-asset options family ({@link ProductSpec})
 * and the fixed-income rates family ({@link RatesProductSpec}), keyed by `family`.
 * The per-family `I` is recovered at the single shell boundary that owns that
 * family's input state. (A registry of differently-typed specs is the one place
 * an erased element type is sound — every consumer pairs a spec with its own
 * matching state.)
 */
export type AnyProductSpec =
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  | ProductSpec<any>
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  | RatesProductSpec<any>;

/** Identity helper that preserves an option spec's `I` at definition while typing the field. */
export function defineProduct<I>(spec: ProductSpec<I>): ProductSpec<I> {
  return spec;
}

/** Identity helper that preserves a rates spec's `I` at definition while typing the field. */
export function defineRatesProduct<I>(spec: RatesProductSpec<I>): RatesProductSpec<I> {
  return spec;
}

/** True for the fixed-income (rates) family — narrows {@link AnyProductSpec} to {@link RatesProductSpec}. */
export function isRatesSpec(spec: AnyProductSpec): spec is RatesProductSpec<unknown> {
  return spec.family === "rates";
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
