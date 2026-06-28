/**
 * capabilityMatrix — the pure resolution logic behind the Admin capability
 * editor. Kept free of React/transport so it is unit-testable in isolation and
 * the matrix component stays a thin view.
 *
 * The model mirrors the server's capability algebra (`celnet-entitlements
 * ::CapabilitySet` + `AuthenticatedUser::capabilities`): the effective set a user
 * holds is `role bundle ∪ grants ∖ denies`, with **deny-wins**. The role bundle
 * is deterministic — `ADMIN` ⇒ grant-all; `TRADER` ⇒ every action except
 * `administer` on both asset classes — so the editor can show the *would-be*
 * role default a deny overrides (the deny-wins visualization) without a round
 * trip. Provenance is in comments only (guardrail #8).
 */

import type {
  Capability,
  CapabilityAction,
  CapabilityAsset,
  UserRole,
} from "../data/contract";
import { CAPABILITY_ACTIONS, CAPABILITY_ASSETS } from "../data/contract";

/** The editable overlay choice for one matrix cell. */
export type OverlayState = "inherit" | "grant" | "deny";

/** A stable key for one action × asset cell. */
export function capKey(action: CapabilityAction, asset: CapabilityAsset): string {
  return `${action} ${asset}`;
}

/** Human-friendly action labels for the matrix rows. */
export const ACTION_LABELS: Record<CapabilityAction, string> = {
  view: "View",
  price: "Price",
  quote_respond: "Quote",
  rfq_respond: "RFQ respond",
  ioi_respond: "IOI respond",
  stream: "Stream",
  execute: "Execute (deal)",
  book: "Book",
  administer: "Administer",
};

/** Human-friendly asset-class labels for the matrix columns. */
export const ASSET_LABELS: Record<CapabilityAsset, string> = {
  fx_options: "FX Options",
  fixed_income: "Fixed Income",
};

/**
 * Whether the user's ROLE bundle alone (before any per-user overlay) admits this
 * action — the documented role→bundle mapping the server enforces:
 * `ADMIN` ⇒ grant-all; `TRADER` ⇒ every action except `administer`, both assets.
 */
export function roleAllows(role: UserRole, action: CapabilityAction): boolean {
  if (role === "ADMIN") return true;
  return action !== "administer";
}

/** The resolved state of one matrix cell under the current overlay choice. */
export interface CellResolution {
  /** Whether the role bundle alone would allow this capability. */
  roleAllows: boolean;
  /** The current overlay choice for the cell. */
  overlay: OverlayState;
  /** The net effect after the overlay (deny-wins): is it allowed? */
  allowed: boolean;
  /** True when a deny is actively overriding a would-be-allowed (role default). */
  denyOverrides: boolean;
}

/**
 * Resolve one cell. `inherit` ⇒ the role default; `grant` ⇒ allowed; `deny` ⇒
 * blocked (deny-wins over both grant and the role default).
 */
export function resolveCell(
  role: UserRole,
  action: CapabilityAction,
  _asset: CapabilityAsset,
  overlay: OverlayState,
): CellResolution {
  const base = roleAllows(role, action);
  const allowed = overlay === "deny" ? false : overlay === "grant" ? true : base;
  // A deny is "winning" only when it actually overrides a would-be-allow.
  const denyOverrides = overlay === "deny" && base;
  return { roleAllows: base, overlay, allowed, denyOverrides };
}

/** Cycle a cell: inherit → grant → deny → inherit. */
export function nextOverlay(state: OverlayState): OverlayState {
  if (state === "inherit") return "grant";
  if (state === "grant") return "deny";
  return "inherit";
}

/** The overlay map keyed by {@link capKey}; absent ⇒ `inherit`. */
export type OverlayMap = Map<string, OverlayState>;

/** Build an overlay map from a server overlay (`grants`/`denies`). Deny-wins if
 * a capability somehow appears in both lists (it never should). */
export function overlayFromCapabilities(
  grants: readonly Capability[],
  denies: readonly Capability[],
): OverlayMap {
  const map: OverlayMap = new Map();
  for (const g of grants) map.set(capKey(g.action, g.asset), "grant");
  for (const d of denies) map.set(capKey(d.action, d.asset), "deny");
  return map;
}

/** The overlay choice at one cell (absent ⇒ `inherit`). */
export function overlayStateAt(
  map: OverlayMap,
  action: CapabilityAction,
  asset: CapabilityAsset,
): OverlayState {
  return map.get(capKey(action, asset)) ?? "inherit";
}

/** Serialize an overlay map back to the wire `grants`/`denies` lists, enumerated
 * in canonical order so the request is deterministic. */
export function overlayToCapabilities(map: OverlayMap): {
  grants: Capability[];
  denies: Capability[];
} {
  const grants: Capability[] = [];
  const denies: Capability[] = [];
  for (const action of CAPABILITY_ACTIONS) {
    for (const asset of CAPABILITY_ASSETS) {
      const state = map.get(capKey(action, asset));
      if (state === "grant") grants.push({ action, asset });
      else if (state === "deny") denies.push({ action, asset });
    }
  }
  return { grants, denies };
}

/**
 * The fully-resolved effective set from a role + overlay map, enumerated over
 * every action × asset (`role bundle ∪ grants ∖ denies`, deny-wins). Mirrors the
 * server's resolution exactly — used for the live preview and as the consistency
 * oracle in tests.
 */
export function resolveEffective(role: UserRole, map: OverlayMap): Capability[] {
  const effective: Capability[] = [];
  for (const action of CAPABILITY_ACTIONS) {
    for (const asset of CAPABILITY_ASSETS) {
      const overlay = overlayStateAt(map, action, asset);
      if (resolveCell(role, action, asset, overlay).allowed) {
        effective.push({ action, asset });
      }
    }
  }
  return effective;
}

/** Whether two overlay maps differ (used for the editor's dirty flag). */
export function overlaysDiffer(a: OverlayMap, b: OverlayMap): boolean {
  if (a.size !== b.size) return true;
  for (const [k, v] of a) {
    if (b.get(k) !== v) return true;
  }
  return false;
}
