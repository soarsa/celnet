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
import { accumulatorSpec } from "./accumulator";
import { lookbackSpec } from "./lookback";
import { windowBarrierSpec } from "./windowBarrier";
import { americanSpec } from "./american";
import { basketSpec } from "./basket";

export type {
  AnyProductSpec,
  AssetClass,
  InputBlockProps,
  ProductBuildCtx,
  ProductGroup,
  ProductSpec,
} from "./types";
export { PRODUCT_GROUP_ORDER, defineProduct, withTenorAndModel } from "./types";

/** Every registered product family, in catalogue order. */
export const PRODUCT_REGISTRY: readonly AnyProductSpec[] = [
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
  accumulatorSpec,
  lookbackSpec,
  windowBarrierSpec,
  americanSpec,
  basketSpec,
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
