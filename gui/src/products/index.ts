/**
 * The product registry (GW2) — the single source of truth for the structurable
 * product catalogue. The ticket shell drives structure selection, the gallery,
 * booking-model resolution and instrument building from this list, so a new
 * product is a new {@link ProductSpec} entry here, not a monolith edit.
 *
 * Order matches the legacy `Structure` catalogue order (within each gallery
 * group). The vanilla + multi-leg strategy family (the leg-ladder) is wired in
 * by the shell integration step; every legless family lives here.
 */
import { PRODUCT_GROUP_ORDER, type AnyProductSpec, type ProductGroup } from "./types";
import {
  vanillaSpec,
  riskReversalSpec,
  strangleSpec,
  straddleSpec,
  seagullSpec,
} from "./strategy";
import { forwardSpec } from "./forward";
import { swapSpec } from "./swap";
import { ndfSpec } from "./ndf";
import { listedFutureOptionSpec } from "./listedFutureOption";
import { singleBarrierSpec } from "./singleBarrier";
import { doubleBarrierSpec } from "./doubleBarrier";
import { digitalSpec } from "./digital";
import { touchSpec } from "./touch";
import { varianceSwapSpec } from "./varianceSwap";
import { volatilitySwapSpec } from "./volatilitySwap";
import { asianSpec } from "./asian";
import { forwardStartSpec } from "./forwardStart";
import { cliquetSpec } from "./cliquet";
import { quantoSpec } from "./quanto";
import { tarfSpec } from "./tarf";
import { pivotSpec } from "./pivot";
import { accumulatorSpec } from "./accumulator";
import { lookbackSpec } from "./lookback";
import { windowBarrierSpec } from "./windowBarrier";
import { americanSpec } from "./american";
import { perpetualSpec } from "./perpetual";
import { basketSpec } from "./basket";
import { crossAssetSpec } from "./crossAsset";
import { oisSpec } from "./ois";
import { irsSpec } from "./irs";
import { fraSpec } from "./fra";
import { bondSpec } from "./bond";

export type {
  AnyProductSpec,
  AssetClass,
  InputBlockProps,
  ProductBuildCtx,
  ProductFamily,
  ProductGroup,
  ProductSpec,
  RatesProductSpec,
} from "./types";
export {
  PRODUCT_GROUP_ORDER,
  defineProduct,
  defineRatesProduct,
  isRatesSpec,
  withTenorAndModel,
} from "./types";
export { oisSpec, OIS_STRUCTURE_ID, type OisInputs } from "./ois";
export { irsSpec, IRS_STRUCTURE_ID, type IrsInputs } from "./irs";
export { fraSpec, FRA_STRUCTURE_ID, type FraInputs } from "./fra";
export { bondSpec, BOND_STRUCTURE_ID, type BondInputs } from "./bond";

// The structuring UI the ticket shell composes the registry with: the grouped
// gallery picker (replaces the flat structure <select>), the payoff-at-expiry
// preview, and the net-structure economics strip.
export { StructureGallery } from "./StructureGallery";
export type { StructureGalleryProps } from "./StructureGallery";
export { PayoffChart } from "./PayoffChart";
export type { PayoffChartProps } from "./PayoffChart";
export { NetStructureStrip } from "./NetStructureStrip";
export type { NetStructureLeg } from "./NetStructureStrip";

/** Every registered product family, in catalogue order. */
export const PRODUCT_REGISTRY: readonly AnyProductSpec[] = [
  vanillaSpec,
  riskReversalSpec,
  strangleSpec,
  straddleSpec,
  seagullSpec,
  // Vanilla on a named listed future (arm 31): futures-measure closed form with
  // the equity-/futures-style premium-margining convention.
  listedFutureOptionSpec,
  // Linear (forwards & swaps): the W2 closed-form DCF products.
  forwardSpec,
  swapSpec,
  ndfSpec,
  singleBarrierSpec,
  doubleBarrierSpec,
  digitalSpec,
  touchSpec,
  varianceSwapSpec,
  volatilitySwapSpec,
  asianSpec,
  forwardStartSpec,
  cliquetSpec,
  quantoSpec,
  tarfSpec,
  // The pivot TRA (arm 32): the TARF generalized with a distinct pivot kink;
  // pivot == strike is the exact TARF slice.
  pivotSpec,
  accumulatorSpec,
  lookbackSpec,
  windowBarrierSpec,
  americanSpec,
  // The perpetual (arm 30): the one tenorless, expiryless product — declares
  // `noExpiry`, so the ticket shell disables the expiry controls for it.
  perpetualSpec,
  basketSpec,
  // Cross-asset (equity / commodity / crypto / metal) vanilla over the W1
  // `Underlying` oneof + settlement-style seam.
  crossAssetSpec,
  // Fixed income (rates): the OIS / vanilla IRS / FRA / cash bond priced through
  // the SAME ticket via the `priceRates` seam (fe-fi-migration #3 +
  // fi-bond-ticket-gui) — a distinct pricing family (RatesProductSpec), each an
  // additive `RatesInstrument` oneof arm, collapsing the standalone FI pricing silo.
  oisSpec,
  irsSpec,
  fraSpec,
  bondSpec,
];

/** Look up a product spec by its structure id, or `undefined` if not registered. */
export function specById(id: string): AnyProductSpec | undefined {
  return PRODUCT_REGISTRY.find((s) => s.id === id);
}

/** The registered specs grouped by gallery group, in canonical group + catalogue order. */
export function registryByGroup(): { group: ProductGroup; specs: AnyProductSpec[] }[] {
  return PRODUCT_GROUP_ORDER.map((group) => ({
    group,
    specs: PRODUCT_REGISTRY.filter((s) => s.group === group),
  })).filter((g) => g.specs.length > 0);
}
