/**
 * The cross-asset CAPABILITY MATRIX — which product-oneof arm each asset class
 * can price END-TO-END — ported to the Excel contract types. This is the
 * build-time guard the task pane reads to disable FX-only arms whenever a
 * cross-asset (equity / commodity / crypto) underlier is active, guiding the
 * trader away from an unpriceable combination BEFORE the request rather than
 * meeting a server error after it.
 *
 * It mirrors the authoritative boundary documented in
 * `docs/EXCEL-INTEGRATION.md §3.3.1` and enforced identically on the other side
 * of the one contract: the server's `price_cross_asset` (crates/celnet-server)
 * and the Excel build-time guard (`excel/src/functions/instrumentSpec.ts`) both
 * refuse a cross-asset exotic. The dim REASON surfaced here uses the SAME honest
 * language as that guard's `ShapingError` — the matrix made visible to the pane,
 * not a second source of truth.
 *
 * This file is the Excel-package port of `gui/src/products/capability.ts`
 * (semantics-identical), adapted to import from
 * `excel/src/contract/contract.ts` (the standalone Excel contract types) rather
 * than from the GUI package. The two files evolve together whenever a new
 * product arm or asset class lands on the one wire contract.
 */

import type { Product, Underlying } from "../contract/contract";

// ---------------------------------------------------------------------------
// local asset-class type (the GUI gets this from products/types.ts; the Excel
// package keeps no equivalent — define it locally, unexported to avoid
// confusion, and re-export under the expected name below)
// ---------------------------------------------------------------------------

/**
 * The trader-facing asset class of an underlier. FX is the origin class; the
 * cross-asset arms (METAL / EQUITY / COMMODITY / CRYPTO) book through the same
 * `Instrument.product` oneof, carrying their identity on `Instrument.underlying`.
 *
 * Defined locally (the Excel package has no `products/types.ts`); semantically
 * identical to `gui/src/products/types.ts AssetClass`.
 */
export type AssetClass = "FX" | "METAL" | "EQUITY" | "COMMODITY" | "CRYPTO";

// ---------------------------------------------------------------------------
// product kind catalogue
// ---------------------------------------------------------------------------

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
 *
 * Mirrors the guard in `excel/src/functions/instrumentSpec.ts`
 * `shapeSpecInstrument` (the build-time `ShapingError` uses names "VANILLA",
 * "PERPETUAL", "FUTUREOPTION" — the canonical trader aliases for these proto arms)
 * and the GUI's `CROSS_ASSET_PRICEABLE` in `gui/src/products/capability.ts`.
 */
export const CROSS_ASSET_PRICEABLE = [
  "vanilla",
  "perpetualOption",
  "listedFutureOption",
] as const satisfies readonly ProductKind[];

/**
 * Which product arms each asset class can price end-to-end. FX & METAL route to
 * the full FX engine (a metal's lease rate is the FX foreign rate), so they price
 * all 24; equity / commodity / crypto price only the carry leaves + agnostic arms.
 */
export const PRICEABLE: Record<AssetClass, ReadonlySet<ProductKind>> = {
  FX: new Set(ALL_PRODUCT_KINDS),
  METAL: new Set(ALL_PRODUCT_KINDS),
  EQUITY: new Set(CROSS_ASSET_PRICEABLE),
  COMMODITY: new Set(CROSS_ASSET_PRICEABLE),
  CRYPTO: new Set(CROSS_ASSET_PRICEABLE),
};

// ---------------------------------------------------------------------------
// asset-class lookup
// ---------------------------------------------------------------------------

/**
 * The trader-facing asset class an underlier arm belongs to, from its
 * `Underlying["kind"]` discriminant. Exhaustive: a new kind on the contract
 * must be classified here before the file compiles.
 */
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

// ---------------------------------------------------------------------------
// priceability predicates
// ---------------------------------------------------------------------------

/** Is a product arm priceable on an asset class (end-to-end, per the matrix)? */
export function isPriceable(kind: ProductKind, cls: AssetClass): boolean {
  return PRICEABLE[cls].has(kind);
}

/** A dimmed verdict: the family is not priceable on the class, with the honest why. */
export interface Dimmed {
  readonly dimmed: true;
  readonly reason: string;
}

/** Lower-case display name of a cross-asset class for the dim reason sentence. */
function className(cls: AssetClass): string {
  // FX stays upper-case; the rest are title-cased ("Equity", "Crypto", …).
  return cls === "FX" ? "FX" : cls.charAt(0) + cls.slice(1).toLowerCase();
}

/**
 * The priceability verdict for a product arm on an asset class. A `Dimmed` result
 * carries the SAME honest sentence the server's `UnsupportedModel` rejection and
 * the Excel build-time guard (`instrumentSpec.ts shapeSpecInstrument`) use —
 * surfaced before the request, never after. The message mirrors:
 *
 *   "a <class> underlier (<label>) supports only VANILLA, PERPETUAL and
 *    FUTUREOPTION — <NAME> is an FX/metal-only product."
 *
 * The exact phrasing keeps parity with `instrumentSpec.ts` so the pane and the
 * function layer speak with one voice to the trader.
 */
export function priceability(kind: ProductKind, cls: AssetClass): "priceable" | Dimmed {
  if (isPriceable(kind, cls)) return "priceable";
  return {
    dimmed: true,
    reason:
      `${className(cls)} underliers support only vanilla, perpetualOption and ` +
      `listedFutureOption (VANILLA / PERPETUAL / FUTUREOPTION) — ` +
      `${kind} is an FX/metal-only product`,
  };
}
