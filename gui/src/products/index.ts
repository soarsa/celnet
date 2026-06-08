/**
 * The product registry (GW2) — the single source of truth for the structurable
 * product catalogue. The ticket shell drives structure selection, the gallery,
 * booking-model resolution and instrument building from this list, so a new
 * product is a new {@link ProductSpec} entry here, not a monolith edit.
 *
 * Families are migrated out of the former `TicketWorkspace` monolith one spec at
 * a time; this index grows as each lands (each gated by `productRegistry.test`).
 */
import { PRODUCT_GROUP_ORDER, type AnyProductSpec, type ProductGroup } from "./types";
import { asianSpec } from "./asian";

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
export const PRODUCT_REGISTRY: readonly AnyProductSpec[] = [asianSpec];

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
