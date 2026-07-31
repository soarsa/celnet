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
  risk_transfer: "Risk transfer",
  simulate: "Simulate",
  administer: "Administer",
  risk_manage: "Manage risk",
  manage_pricing: "Manage pricing",
  manage_liquidity: "Manage liquidity",
  view_analytics: "View analytics",
  hedge: "Auto-hedge",
};

/**
 * The actions the default `TRADER` role bundle HOLDS BACK — every action NOT in
 * this set is conferred by the role on both asset classes; the ones listed here
 * require an explicit per-user (or edited-role) grant. Mirrors the server's
 * `config/identity.rs::default_trader_bundle`, which withholds `Administer`,
 * `RiskTransfer`, and the three management authorities `RiskManage` /
 * `ManagePricing` / `ManageLiquidity` and the cross-asset read `ViewAnalytics`
 * (`docs/PERMISSIONS-GRANULAR-REVIEW.md` §5; `config/identity.rs`).
 */
export const TRADER_HELD_BACK_ACTIONS: ReadonlySet<CapabilityAction> =
  new Set<CapabilityAction>([
    "administer",
    "risk_transfer",
    "risk_manage",
    "manage_pricing",
    "manage_liquidity",
    "view_analytics",
    "hedge",
  ]);

/** Human-friendly asset-class labels for the matrix columns. */
export const ASSET_LABELS: Record<CapabilityAsset, string> = {
  fx_options: "FX Options",
  fixed_income: "Fixed Income",
};

/**
 * Whether the user's ROLE bundle alone (before any per-user overlay) admits this
 * action — the documented role→bundle mapping the server enforces (`config/
 * identity.rs::default_trader_bundle`): `ADMIN` ⇒ grant-all; `TRADER` ⇒ every
 * action except the narrow explicitly-granted authorities in
 * {@link TRADER_HELD_BACK_ACTIONS} (`administer`, `risk_transfer`, and the three
 * management caps `risk_manage` / `manage_pricing` / `manage_liquidity`), on both
 * asset classes.
 */
export function roleAllows(role: UserRole, action: CapabilityAction): boolean {
  if (role === "ADMIN") return true;
  return !TRADER_HELD_BACK_ACTIONS.has(action);
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

/**
 * Membership test against a user's effective capability set: does `caps` contain
 * the capability `action × asset`? This is the single selector the client gates
 * affordances on — the effective set is the server's authoritative `role bundle ∪
 * grants ∖ denies` (`LoginResult.capabilities`), so a `false` here mirrors a
 * server deny exactly. An empty set denies everything (a coherent deny-by-default
 * when the caller's capabilities are unknown). UX-only: the server still enforces.
 */
export function can(
  caps: readonly Capability[],
  action: CapabilityAction,
  asset: CapabilityAsset,
): boolean {
  return caps.some((c) => c.action === action && c.asset === asset);
}

/** The asset-class adjective used in a human-readable denial tooltip. */
const ASSET_ADJECTIVE: Record<CapabilityAsset, string> = {
  fx_options: "FX-options",
  fixed_income: "fixed-income",
};

/** The gerund phrase for each action, completed with the asset adjective. */
const ACTION_PHRASE: Record<CapabilityAction, (asset: string) => string> = {
  view: (a) => `viewing ${a} data`,
  price: (a) => `requesting ${a} prices`,
  quote_respond: (a) => `responding to ${a} dealer quote requests`,
  rfq_respond: (a) => `responding to ${a} RFQs`,
  ioi_respond: (a) => `responding to ${a} IOIs`,
  stream: (a) => `streaming live ${a} prices`,
  execute: (a) => `executing ${a} trades`,
  book: (a) => `booking ${a} positions`,
  risk_transfer: (a) => `transferring ${a} risk between books`,
  simulate: (a) => `using the ${a} counterparty simulator`,
  administer: () => `administering Celnet`,
  risk_manage: (a) => `managing ${a} risk portfolios, routing and the risk dashboard`,
  manage_pricing: (a) => `managing ${a} pricing groups and session tiering`,
  manage_liquidity: (a) => `managing ${a} liquidity connections and aggregated books`,
  view_analytics: (a) => `viewing the ${a} client-flow analytics`,
  hedge: (a) => `authoring ${a} auto-hedge policies, thresholds and the hedge monitor`,
};

/**
 * The explanatory tooltip shown on an affordance the signed-in user is NOT
 * permitted to use — e.g. "Your permissions don't allow executing fixed-income
 * trades." Gated controls are disabled (never hidden) and carry this text so the
 * denial is discoverable and explained, not a silent grey-out.
 */
export function capabilityDenialTitle(
  action: CapabilityAction,
  asset: CapabilityAsset,
): string {
  return `Your permissions don't allow ${ACTION_PHRASE[action](ASSET_ADJECTIVE[asset])}.`;
}

/** Whether two overlay maps differ (used for the editor's dirty flag). */
export function overlaysDiffer(a: OverlayMap, b: OverlayMap): boolean {
  if (a.size !== b.size) return true;
  for (const [k, v] of a) {
    if (b.get(k) !== v) return true;
  }
  return false;
}

// ---------------------------------------------------------------------------
// Component → capability mapping (the Permissions page's primary model)
//
// The capability algebra above is action × asset (the wire contract). A TRADER,
// however, thinks in COMPONENTS — "can this user use the Ticket / the Rates Book
// / Administration". This table is the pure, testable bridge: each UI component
// declares the capability ACTIONS that constitute its Read and its Write access,
// on its asset class. Read = `view` on the component's asset (the right to SEE
// it); Write = the component's own action(s) on that asset (an empty list ⇒ a
// read-only component, no write affordance). The two grid toggles are PROJECTIONS
// over these capability sets, and every edit still round-trips through the same
// deny-wins overlay algebra the server enforces — the component view is sugar
// over the one contract, never a parallel model.
// ---------------------------------------------------------------------------

/** The grid section a component is grouped under (mirrors the product domains). */
export type ComponentSection =
  | "fx_options"
  | "fixed_income"
  | "analytics"
  | "administration";

/**
 * One trader-facing component and the capabilities that constitute its Read and
 * Write access. The Administration row is the cross-asset exception: it governs
 * `administer` on BOTH asset classes as a single toggle (Read == Write).
 *
 * NOTE (fe-fi-migration #6): the RAIL collapsed its duplicate FX/FI rows into one
 * class-parametric row each, but the Permissions grid is the per-ASSET-CLASS
 * CAPABILITY editor — its components enumerate the distinct capabilities an admin
 * grants (e.g. `price·fixed_income` via the `rates` component, `book·fixed_income`
 * via `ratesbook`), so it deliberately stays per-asset and is NOT collapsed:
 * collapsing it would STRAND those per-asset capabilities. A component id here is a
 * capability-grouping key — most map to a rail workspace, some (`rates`/`curve`/
 * `ratesrisk`/`deals`/`ratesbook`) now map to a lens/product-family reached via a
 * SHARED class-parametric workspace, and `simulator`/`administration` are non-rail.
 */
export interface ComponentAccess {
  /** Stable capability-grouping id (usually a rail workspace id; see the note above). */
  id: string;
  /** Human label for the grid row. */
  label: string;
  /** The grid section the component lives under. */
  section: ComponentSection;
  /** The asset class(es) the component's capabilities apply to (both for Administration). */
  assets: readonly CapabilityAsset[];
  /** The action(s) constituting READ access (`view`, except the cross-asset admin row). */
  readActions: readonly CapabilityAction[];
  /** The action(s) constituting WRITE access (empty ⇒ read-only component). */
  writeActions: readonly CapabilityAction[];
}

/**
 * THE component → capability spec. Read is `view` for the component's asset;
 * Write is the component's action(s) for that asset; read-only components have an
 * empty write set. Administration is cross-asset `administer` (one toggle, both
 * assets, Read == Write).
 */
export const COMPONENT_ACCESS: readonly ComponentAccess[] = [
  // FX Options (asset `fx_options`).
  {
    id: "ticket",
    label: "Ticket",
    section: "fx_options",
    assets: ["fx_options"],
    readActions: ["view"],
    writeActions: ["price", "execute"],
  },
  {
    id: "stream",
    label: "Stream",
    section: "fx_options",
    assets: ["fx_options"],
    readActions: ["view"],
    writeActions: ["stream", "execute"],
  },
  {
    id: "surface",
    label: "Surface",
    section: "fx_options",
    assets: ["fx_options"],
    readActions: ["view"],
    writeActions: [],
  },
  {
    id: "risk",
    label: "Risk",
    section: "fx_options",
    assets: ["fx_options"],
    readActions: ["view"],
    writeActions: [],
  },
  // Fixed Income (asset `fixed_income`).
  {
    id: "rates",
    label: "Rates",
    section: "fixed_income",
    assets: ["fixed_income"],
    readActions: ["view"],
    writeActions: ["price"],
  },
  {
    id: "curve",
    label: "Curve",
    section: "fixed_income",
    assets: ["fixed_income"],
    readActions: ["view"],
    writeActions: [],
  },
  {
    id: "ratesrisk",
    label: "Rates Risk",
    section: "fixed_income",
    assets: ["fixed_income"],
    readActions: ["view"],
    writeActions: [],
  },
  {
    id: "quoting",
    label: "Quoting",
    section: "fixed_income",
    assets: ["fixed_income"],
    readActions: ["view"],
    writeActions: ["quote_respond", "rfq_respond", "ioi_respond", "execute"],
  },
  {
    id: "deals",
    label: "Deals",
    section: "fixed_income",
    assets: ["fixed_income"],
    readActions: ["view"],
    writeActions: [],
  },
  {
    id: "ratesbook",
    label: "Rates Book",
    section: "fixed_income",
    assets: ["fixed_income"],
    readActions: ["view"],
    writeActions: ["book"],
  },
  {
    id: "book",
    label: "Book",
    section: "fixed_income",
    assets: ["fixed_income"],
    readActions: ["view"],
    writeActions: [],
  },
  {
    id: "simulator",
    label: "Simulator",
    section: "fixed_income",
    assets: ["fixed_income"],
    readActions: ["view"],
    writeActions: ["simulate"],
  },
  // FI management authorities (docs/PERMISSIONS-GRANULAR-REVIEW.md §4.3): one
  // trader-friendly Write toggle per granular management cap, so an admin can grant
  // it from the component grid (the raw CapabilityMatrix already lists every action
  // row automatically). Read = `view·FI` (shared with the other FI reads); Write =
  // exactly the one management action the cap authorizes.
  {
    id: "riskmanage",
    label: "Manage Risk",
    section: "fixed_income",
    assets: ["fixed_income"],
    readActions: ["view"],
    writeActions: ["risk_manage"],
  },
  {
    id: "managepricing",
    label: "Manage Pricing",
    section: "fixed_income",
    assets: ["fixed_income"],
    readActions: ["view"],
    writeActions: ["manage_pricing"],
  },
  {
    id: "manageliquidity",
    label: "Manage Liquidity",
    section: "fixed_income",
    assets: ["fixed_income"],
    readActions: ["view"],
    writeActions: ["manage_liquidity"],
  },
  // Analytics — the cross-asset client-flow / P&L-attribution surface
  // (docs/ANALYTICS-REQUIREMENTS.md §11.1a). A management-sensitive READ on BOTH
  // assets (holding it on either admits — the server gate is a cross-product OR),
  // held back from the default trader bundle. Read-only (no write affordance).
  {
    id: "analytics",
    label: "Analytics",
    section: "analytics",
    assets: CAPABILITY_ASSETS,
    readActions: ["view_analytics"],
    writeActions: [],
  },
  // Administration — a single cross-asset toggle governing `administer` on BOTH
  // assets together. Read == Write (the same capability).
  {
    id: "administration",
    label: "Administration",
    section: "administration",
    assets: CAPABILITY_ASSETS,
    readActions: ["administer"],
    writeActions: ["administer"],
  },
];

/** The grid sections in display order (each maps to a product domain). */
export const COMPONENT_SECTIONS: readonly { id: ComponentSection; label: string }[] = [
  { id: "fx_options", label: "FX Options" },
  { id: "fixed_income", label: "Fixed Income" },
  { id: "analytics", label: "Analytics" },
  { id: "administration", label: "Administration" },
];

/** Expand `actions × assets` into the flat capability list (canonical order). */
function capsFor(
  actions: readonly CapabilityAction[],
  assets: readonly CapabilityAsset[],
): Capability[] {
  const out: Capability[] = [];
  for (const action of actions) {
    for (const asset of assets) out.push({ action, asset });
  }
  return out;
}

/** The component's READ capability set (`readActions × assets`). */
export function componentReadCaps(c: ComponentAccess): Capability[] {
  return capsFor(c.readActions, c.assets);
}

/** The component's WRITE capability set (`writeActions × assets`); empty ⇒ read-only. */
export function componentWriteCaps(c: ComponentAccess): Capability[] {
  return capsFor(c.writeActions, c.assets);
}

/**
 * The distinct capabilities the component's advanced view edits — its read set
 * unioned with its write set, de-duplicated, in canonical order.
 */
export function componentAdvancedCaps(c: ComponentAccess): Capability[] {
  const seen = new Set<string>();
  const out: Capability[] = [];
  for (const cap of [...componentReadCaps(c), ...componentWriteCaps(c)]) {
    const k = capKey(cap.action, cap.asset);
    if (seen.has(k)) continue;
    seen.add(k);
    out.push(cap);
  }
  return out;
}

/** Whether the component has no write actions (its Write toggle is disabled). */
export function isReadOnlyComponent(c: ComponentAccess): boolean {
  return c.writeActions.length === 0;
}

/** A projection toggle's resolved state over a capability set. */
export type ToggleState = "on" | "off" | "mixed";

/**
 * Resolve a toggle that PROJECTS over `caps` under the deny-wins algebra: `on`
 * when every capability is effective, `off` when none is, `mixed` when some are
 * and some are not. An empty set (a read-only component's write set) resolves to
 * `off` — the disabled affordance. Mirrors the server's effective resolution per
 * cell, so the toggle reflects exactly what the user can do.
 */
export function toggleState(
  role: UserRole,
  overlay: OverlayMap,
  caps: readonly Capability[],
): ToggleState {
  if (caps.length === 0) return "off";
  let anyOn = false;
  let anyOff = false;
  for (const c of caps) {
    const allowed = resolveCell(
      role,
      c.action,
      c.asset,
      overlayStateAt(overlay, c.action, c.asset),
    ).allowed;
    if (allowed) anyOn = true;
    else anyOff = true;
  }
  if (anyOn && anyOff) return "mixed";
  return anyOn ? "on" : "off";
}

/**
 * The overlay state a toggle CLICK projects across its whole capability set: an
 * `on` toggle turns OFF (deny all — deny-wins, visually obvious); an `off` OR
 * `mixed` toggle turns ON (grant all — a mixed toggle resolves to all-on first).
 */
export function toggleTarget(state: ToggleState): OverlayState {
  return state === "on" ? "deny" : "grant";
}

/**
 * Immutably set every capability in `caps` to `state` in a copy of `map`
 * (`inherit` ⇒ remove the cell so it falls back to the role default). The single
 * projection primitive both grid toggles and tests drive.
 */
export function setOverlayFor(
  map: OverlayMap,
  caps: readonly Capability[],
  state: OverlayState,
): OverlayMap {
  const next = new Map(map);
  for (const c of caps) {
    const key = capKey(c.action, c.asset);
    if (state === "inherit") next.delete(key);
    else next.set(key, state);
  }
  return next;
}

/** One asset class's role-baseline count: `allowed` of `total` actions held. */
export interface RoleAssetSummary {
  asset: CapabilityAsset;
  allowed: number;
  total: number;
}

/**
 * The honest ROLE-BASELINE capability summary for a user row: for each asset
 * class, how many of the {@link CAPABILITY_ACTIONS} the role holds with NO
 * per-user overlay (`resolveEffective(role, ∅)`). This is the baseline the role
 * confers (admin ⇒ 15/15 both; trader ⇒ 9/15 both — every action except the six
 * held-back authorities in {@link TRADER_HELD_BACK_ACTIONS}); the full,
 * overlay-adjusted effective set stays reachable through the per-user Permissions
 * editor. Deliberately overlay-free so a compact roster chip never misrepresents a
 * per-user grant/deny as a role property.
 */
export function roleBaselineSummary(role: UserRole): RoleAssetSummary[] {
  const effective = resolveEffective(role, new Map());
  const total = CAPABILITY_ACTIONS.length;
  return CAPABILITY_ASSETS.map((asset) => ({
    asset,
    allowed: effective.filter((c) => c.asset === asset).length,
    total,
  }));
}
