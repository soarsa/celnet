/**
 * The cross-asset CAPABILITY MATRIX — which product-oneof arm each asset class can
 * price END-TO-END — made executable. This is the single source the discovery layer
 * (the structure gallery), the class-aware analytics, and the Excel task-pane all
 * read, so the trader is GUIDED away from an unpriceable combination BEFORE the
 * request rather than meeting a server error after it.
 *
 * It mirrors the authoritative boundary documented in
 * `docs/clients/EXCEL-INTEGRATION.md §3.3.1` and enforced identically on the other side of
 * the one contract: the server's `price_cross_asset` (crates/celnet-server) and the
 * Excel build-time guard (`excel/src/functions/instrumentSpec.ts`) both refuse a
 * cross-asset exotic. The dim REASON surfaced here is the same honest sentence those
 * guards reject with — the matrix made visible, not a second source of truth.
 *
 * Rationale (the rule-10 "evolve, don't churn" choice): a single pure module keyed
 * by `(asset class × product kind)` keeps all 27 per-family spec files untouched and
 * gives exactly ONE place to port to the Excel client.
 */
import type { Underlying, Product } from "../data/contract";
import type { AssetClass, ProductFamily } from "./types";

/** A product-oneof arm (the wire `Instrument.product` discriminant). */
export type ProductKind = Product["kind"];

/**
 * Every product-oneof arm, in catalogue order — the 24 families on the contract.
 * Kept as the literal source so a new arm is a compile error here until classified
 * (the `satisfies` below makes an out-of-set or missing kind fail typecheck).
 */
export const ALL_PRODUCT_KINDS = [
  "vanilla",
  "strategy",
  "singleBarrier",
  "doubleBarrier",
  "digital",
  "touch",
  "varianceSwap",
  "volatilitySwap",
  "asianOption",
  "forwardStart",
  "cliquet",
  "quanto",
  "tarf",
  "pivot",
  "accumulator",
  "lookback",
  "windowBarrier",
  "american",
  "basket",
  "fxForward",
  "fxSwap",
  "ndf",
  "perpetualOption",
  "listedFutureOption",
] as const satisfies readonly ProductKind[];

/**
 * The cross-asset (equity / commodity / crypto) priceable set: the generalized
 * cost-of-carry leaf (`vanilla`) plus the two asset-class-AGNOSTIC arms — the
 * perpetual stationary-ODE (`perpetualOption`) and Black-76 on a quoted future
 * (`listedFutureOption`), whose carry is embodied by the contract identity. Every
 * other arm is an FX/metal-only engine. (`price_cross_asset` accepts exactly these.)
 */
export const CROSS_ASSET_PRICEABLE = [
  "vanilla",
  "perpetualOption",
  "listedFutureOption",
] as const satisfies readonly ProductKind[];

/**
 * Which product arms each asset class can price end-to-end. FX & METAL route to the
 * full FX engine (a metal's lease rate is the FX foreign rate), so they price all 24;
 * equity/commodity/crypto price only the carry leaves + agnostic arms.
 */
export const PRICEABLE: Record<AssetClass, ReadonlySet<ProductKind>> = {
  FX: new Set(ALL_PRODUCT_KINDS),
  METAL: new Set(ALL_PRODUCT_KINDS),
  EQUITY: new Set(CROSS_ASSET_PRICEABLE),
  COMMODITY: new Set(CROSS_ASSET_PRICEABLE),
  CRYPTO: new Set(CROSS_ASSET_PRICEABLE),
};

/** The trader-facing asset class an underlier belongs to, from its `Underlying` arm. */
export function classForUnderlying(kind: Underlying["kind"]): AssetClass {
  switch (kind) {
    case "fx":
      return "FX";
    case "metal":
      return "METAL";
    case "equity":
      return "EQUITY";
    case "commodity":
      return "COMMODITY";
    case "digitalAsset":
      return "CRYPTO";
  }
}

/** Is a product arm priceable on an asset class (end-to-end, per the matrix)? */
export function isPriceable(kind: ProductKind, cls: AssetClass): boolean {
  return PRICEABLE[cls].has(kind);
}

/** A dimmed verdict: the family is not priceable on the class, with the honest why. */
export interface Dimmed {
  readonly dimmed: true;
  readonly reason: string;
}

/** Lower-case, human display name of an asset class for the dim reason. */
function className(cls: AssetClass): string {
  return cls === "FX" ? "FX" : cls.charAt(0) + cls.slice(1).toLowerCase();
}

/**
 * The priceability verdict for a product arm on an asset class. A `Dimmed` result
 * carries the SAME honest sentence the server's `UnsupportedModel` rejection and the
 * Excel build-time guard use — surfaced before the request, never after.
 */
export function priceability(kind: ProductKind, cls: AssetClass): "priceable" | Dimmed {
  if (isPriceable(kind, cls)) return "priceable";
  return {
    dimmed: true,
    reason:
      `${className(cls)} prices only vanilla, perpetual and listed-future-option ` +
      `(the cost-of-carry leaves) — this is an FX/metal-only product`,
  };
}

/**
 * The default builder classes for an FX-native product spec: the FX engine, which
 * also prices a metal-vs-fiat pair (a metal's lease rate is the FX foreign rate).
 * Cross-asset specs widen this via `ProductSpec.applicableClasses`.
 */
export const DEFAULT_BUILDER_CLASSES: readonly AssetClass[] = ["FX", "METAL"];

/**
 * Every asset class — the `applicableClasses` for the asset-class-AGNOSTIC arms
 * (the perpetual stationary-ODE + Black-76 on a quoted future), which the server's
 * `price_cross_asset` prices END-TO-END on equity / commodity / crypto exactly as on
 * FX / metal (`CROSS_ASSET_PRICEABLE` ⊇ these two arms for every cross-asset class).
 * Declaring it makes the gallery show DISTINCT perpetual / future-option cards on a
 * cross-asset underlier (the trader can structure them there), while the spec's
 * `toInstrument` carries the active `Underlying` via {@link crossAssetOverlayFor}.
 */
export const ALL_ASSET_CLASSES: readonly AssetClass[] = [
  "FX",
  "METAL",
  "EQUITY",
  "COMMODITY",
  "CRYPTO",
];

/**
 * A discovery-gallery card's state for the active asset class:
 *  - `available`   — this spec builds a priceable instrument for the class (selectable);
 *  - `Dimmed`      — no spec of this product arm prices on the class (FX/metal-only),
 *                    shown dimmed with the honest reason so the matrix is VISIBLE;
 *  - `hidden`      — another spec of the SAME arm is the builder for this class, so
 *                    this one would be a confusing duplicate (e.g. the FX vanilla
 *                    card on an equity underlier, where the cross-asset vanilla builds).
 */
export type CardState = "available" | Dimmed | "hidden";

/** The minimal spec shape `galleryCardStates` reads (the registry's `AnyProductSpec`). */
export interface SpecApplicability {
  readonly id: string;
  /**
   * The `Instrument` product-oneof arm — present ONLY on the option family. The
   * fixed-income (`rates`) family has no `Instrument` arm (it prices via the OIS
   * `priceRates` seam), so this is absent for it and it is handled specially below.
   */
  readonly kind?: ProductKind;
  /** The pricing family — absent ⇒ the option family; `"rates"` ⇒ fixed income. */
  readonly family?: ProductFamily;
  readonly applicableClasses?: readonly AssetClass[];
}

const builderClasses = (s: SpecApplicability): readonly AssetClass[] =>
  s.applicableClasses ?? DEFAULT_BUILDER_CLASSES;

/**
 * Partition a spec catalogue for an asset class into per-card states (pure). A spec
 * is AVAILABLE iff it builds for the class; otherwise HIDDEN when another spec of the
 * same product arm IS available on the class (no duplicate card), else DIMMED with
 * the honest capability reason — so a trader on an equity/commodity/crypto underlier
 * sees exactly what they can build and learns why the FX/metal-only families can't.
 */
export function galleryCardStates(
  specs: readonly SpecApplicability[],
  cls: AssetClass,
): Map<string, CardState> {
  const states = new Map<string, CardState>();
  // FX & METAL route to the FULL FX engine (a metal's lease rate is the FX foreign
  // rate), so every registered family is available — no dimming, the catalogue is
  // unchanged from the class-unaware gallery. Dimming is reserved for the true
  // cross-asset classes whose engines are the narrower cost-of-carry leaves.
  if (cls === "FX" || cls === "METAL") {
    for (const s of specs) states.set(s.id, "available");
    return states;
  }
  // Equity / commodity / crypto: AVAILABLE iff this spec builds for the class;
  // else HIDDEN when a sibling spec of the same arm IS the builder here (no
  // duplicate card — e.g. the FX vanilla hidden behind the cross-asset vanilla);
  // else DIMMED with the honest capability reason (the FX/metal-only families).
  // The fixed-income (rates) family is NOT priced on an FX asset class at all (it
  // prices the USD-SOFR OIS via `priceRates`, gated by the `fixed_income`
  // license/entitlement at the price action, not by the active FX underlier), so
  // it is AVAILABLE on every class — never dimmed by the cross-asset matrix.
  const availableKinds = new Set(
    specs
      .filter((s) => s.family !== "rates" && s.kind !== undefined && builderClasses(s).includes(cls))
      .map((s) => s.kind as ProductKind),
  );
  for (const s of specs) {
    const kind = s.kind;
    if (s.family === "rates" || kind === undefined) {
      states.set(s.id, "available");
    } else if (builderClasses(s).includes(cls)) {
      states.set(s.id, "available");
    } else if (availableKinds.has(kind)) {
      states.set(s.id, "hidden");
    } else {
      const p = priceability(kind, cls);
      states.set(s.id, p === "priceable" ? "hidden" : p);
    }
  }
  return states;
}
