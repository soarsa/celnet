/**
 * commands.ts — the SINGLE source-of-truth command + keybinding registry (GW1).
 *
 * Before GW1 the keyboard grammar lived in THREE places that could drift: the
 * Shell's inline `onKey` handler (what was honoured), `lib/shortcuts.ts` (what was
 * advertised in the `?` cheatsheet), and the hard-coded CommandPalette list. This
 * registry collapses them: every binding — its chord, group, label, and the data-
 * driven workspace jumps `⌘1..n` — is declared ONCE here. The Shell dispatches
 * FROM this registry, the `?` cheatsheet is a PROJECTION of it (`lib/shortcuts.ts`
 * re-exports `cheatsheet()`), and the palette is the runnable command list.
 *
 * SINGLE-SOURCE INVARIANT (GW-FOUNDATION-PLAN §2, command-registry row): every key
 * the Shell honours is a binding here and vice-versa (no orphan handler, no dead
 * advertised chord); chords are collision-free; the cheatsheet == the registry
 * projection. `test/commands.test.ts` asserts these against the registry, and the
 * disjoint chord-uniqueness check is reached without replaying the dispatcher.
 *
 * Split of concerns:
 *   - `COMMAND_META`  — static metadata (id, keys, group, label, kind). No app
 *                       state, so the cheatsheet + tests read it without a render.
 *   - `buildCommands` — binds the static registry to live app actions, yielding the
 *                       runnable `Command[]` the palette + Shell dispatch.
 */

import { CAPABILITY_ASSETS, type CapabilityAction, type CapabilityAsset } from "../data/contract";
import type { Command } from "../components/CommandPalette";

/**
 * Workspace ids the rail exposes (kept in sync with `AppContext.WorkspaceId`).
 *
 * fe-fi-migration #6 (capstone): the FX-vs-FI DOMAIN-TAB split is retired for a
 * SINGLE class-parametric rail. The duplicated FX/FI rows that routed to the same
 * shared workspace under different lenses are COLLAPSED into ONE capability row
 * each — `ratesrisk`→`risk`, `curve`→`surface` (Market Data), `deals`+`ratesbook`
 * →`book`, `rates`→`ticket` — with the ASSET CLASS chosen by the scope/underlier +
 * the license lens INSIDE the workspace (built by #1–#4), not by a duplicate rail
 * row. `quoting` had no FX twin and is unchanged. No capability is lost: a rates
 * ticket is priced through the shared `ticket` (its rates product family), an FI
 * curve through the shared `surface` (its rates lens), an FI book through the
 * shared `book` (its positions lens), FI risk through the shared `risk` (its rates
 * lens) — all reachable under a fixed-income scope/license.
 */
export type WorkspaceId =
  | "ticket"
  | "stream"
  | "surface"
  | "risk"
  | "book"
  | "quoting"
  | "fistreaming"
  | "aggbook"
  | "tiering"
  | "riskbooks"
  | "riskdashboard"
  | "riskrouting"
  | "risktransfer"
  | "transferinbox"
  | "transferaudit"
  | "xva"
  | "excel"
  | "connections"
  | "admin"
  | "permissions"
  | "pricinggroups"
  | "refdata";

/** A logical grouping of related commands (sections the cheatsheet + palette use). */
export type CommandGroup = "Global" | "Workspace" | "Scope" | "Action";

/** How a command's chord is matched against a keydown (the Shell's grammar). */
export type ChordKind =
  | { kind: "meta"; key: string } // ⌘/Ctrl + key (case-insensitive single char)
  | { kind: "metaDigit" } // ⌘/Ctrl + 1..n (workspace jumps; n = rail length)
  | { kind: "plain"; key: string }; // a bare key (e.g. `?`), guarded vs text fields

/** Static metadata for one command: identity, chord, section, cheatsheet label. */
export interface CommandMeta {
  /** Stable id (test/aria keying + the palette command id). */
  id: string;
  /** Display chord tokens (rendered as <kbd>); empty ⇒ palette-only, no global key. */
  keys: string[];
  /** The section the command belongs to. */
  group: CommandGroup;
  /** Human-readable action label (the cheatsheet + palette title share this). */
  label: string;
  /** How the Shell matches the chord; absent ⇒ no global key handler. */
  chord?: ChordKind;
}

/**
 * The rail workspaces, in order — ONE single class-parametric rail (fe-fi-migration
 * #6). The data-driven rail: the `⌘N` hint and the `metaDigit` chord are DERIVED
 * from this order, so adding a view needs only a row here — its `⌘N` lights up for
 * free (`⌘1..⌘9`, then `⌘0` for a tenth; the eleventh+ are palette-only). Glyph
 * fix: Book is `▤` (a ledger), freeing `Σ` for sum/vega-ladder use exclusively (one
 * glyph, one meaning).
 *
 * Each row declares the asset class(es) it serves via {@link workspaceAssets}:
 *   • CROSS-ASSET (class-parametric) rows list BOTH classes — the class is chosen
 *     INSIDE the workspace by the scope/underlier + the license lens (#1–#4), never
 *     by a duplicate rail row. `ticket` (Price), `surface` (Market Data), `risk`,
 *     and `book` each span FX + FI.
 *   • SINGLE-ASSET rows list one class: `stream` (FX price streaming) and `quoting`
 *     (fixed-income dealer quoting — no FX twin). `xva`/`excel` stay FX-scoped (no
 *     FI reconciliation was built for them, so their entitlement is unchanged).
 *   • ADMIN/ops rows list NONE — gated by `isAdmin`, with no license concept.
 *   • MANAGEMENT rows carry a fine-grained `viewCap` (Risk Portfolios/Routing/
 *     Dashboard → `risk_manage·FI`; Tiering/Pricing Groups → `manage_pricing·FI`):
 *     visible ONLY to a holder of that capability (docs/PERMISSIONS-GRANULAR-REVIEW.md
 *     §4). They still declare their served asset for domain-tab placement.
 * The rail is driven by scope/underlier + license, NOT by an FX/FI domain tab.
 */
export const RAIL: readonly {
  id: WorkspaceId;
  glyph: string;
  label: string;
  /**
   * A one-line rail subtitle saying what THIS surface IS — the at-a-glance
   * disambiguation for rows a trader otherwise confuses (the "Risk" vs "Book" vs
   * "Risk Portfolios" vs "Risk Dashboard" vs "Agg Book" family; see
   * docs/FI-BOOK-CONCEPTS.md). Rendered under the label in the rail and folded into
   * the hover tooltip. Optional — rows that are already self-explanatory omit it.
   */
  subtitle?: string;
  /** The asset class(es) this workspace serves (see {@link workspaceAssets}). */
  assets: readonly CapabilityAsset[];
  /**
   * The FINE-GRAINED capability that makes this rail entry VISIBLE at all
   * (docs/PERMISSIONS-GRANULAR-REVIEW.md §4). When present, {@link workspaceAccessible}
   * gates the whole surface on `auth.can(viewCap.action, viewCap.asset)` — a user
   * without it never sees the entry (not merely a disabled control). Used for the FI
   * management surfaces (Risk Portfolios/Routing/Dashboard → `risk_manage·FI`;
   * Tiering/Pricing Groups → `manage_pricing·FI`) so only the granted risk/pricing
   * managers see them. Absent ⇒ the default gate (admin-only ⇒ isAdmin; else view on
   * any served asset). Admin holds `grant_all`, so admins see every viewCap surface;
   * `can` is permissive signed-out, so pre-login discovery is unchanged.
   */
  viewCap?: { action: CapabilityAction; asset: CapabilityAsset };
}[] = [
  // Trading capabilities — class chosen by scope/underlier + lens INSIDE the pane.
  // Ticket (Price) is FX-only: FI is booked/streamed through the Streaming hub +
  // Quoting rows, so the Ticket row derives to the FX Options tab ONLY (it no
  // longer appears under Fixed Income — FI pricing lives on the Streaming surface).
  { id: "ticket", glyph: "⌁", label: "Ticket", assets: ["fx_options"] },
  { id: "stream", glyph: "≋", label: "Stream", assets: ["fx_options"] },
  // FI live streaming (bond + swap prices) with the instrument selector + RFS
  // request sidebar — a single-asset FI row (never FX), and the PRIMARY Fixed
  // Income surface, so it leads the FI rail (top of railForDomain("fixed_income")).
  { id: "fistreaming", glyph: "⇉", label: "Streaming", subtitle: "Live bond & swap prices", assets: ["fixed_income"] },
  // FI aggregated-book live composite view (ADR-0022): consolidated best bid/offer
  // across a book's inbound liquidity members — a single-asset FI read surface.
  { id: "aggbook", glyph: "◫", label: "Agg Book", subtitle: "LP-aggregated prices", assets: ["fixed_income"] },
  // Tiering: the roster of FIX sessions → the pricing group applied to each. A
  // ManagePricing·FI surface (assign a group = a group-membership edit) — gated at
  // the rail on `manage_pricing·FI` so only the FI pricing desk sees it.
  { id: "tiering", glyph: "⚖", label: "Tiering", subtitle: "Per-session pricing", assets: ["fixed_income"], viewCap: { action: "manage_pricing", asset: "fixed_income" } },
  // FI Risk routing (docs/FI-RISK-ROUTING-REQUIREMENTS.md): the hierarchical risk-
  // portfolio tree editor and the per-portfolio risk DASHBOARD. Single-asset FI
  // rows gated at the rail on the granular `risk_manage·FI` capability
  // (docs/PERMISSIONS-GRANULAR-REVIEW.md §4) — a firm risk-control function distinct
  // from super-admin, so a risk lead sees + edits these WITHOUT full Administer, and
  // an ordinary FI trader no longer sees them at all. USER-FACING name "Risk
  // Portfolios"; the wire type stays `RiskBookDef`/`riskbooks` (rename is UI-only —
  // see docs/FI-BOOK-CONCEPTS.md).
  { id: "riskbooks", glyph: "❦", label: "Risk Portfolios", subtitle: "Risk buckets + limits", assets: ["fixed_income"], viewCap: { action: "risk_manage", asset: "fixed_income" } },
  { id: "riskdashboard", glyph: "◉", label: "Risk Dashboard", subtitle: "Routed-risk roll-up", assets: ["fixed_income"], viewCap: { action: "risk_manage", asset: "fixed_income" } },
  // Risk Routing: the ordered rules table that routes each fill's risk into a desk's
  // risk portfolio. Rail-gated + edited on `risk_manage·FI` (was the overloaded
  // `quote_respond·FI` stand-in). Single-asset FI row.
  { id: "riskrouting", glyph: "⑃", label: "Risk Routing", subtitle: "Fill → portfolio rules", assets: ["fixed_income"], viewCap: { action: "risk_manage", asset: "fixed_income" } },
  // FI Risk transfer (docs/RISK-TRANSFER-REQUIREMENTS.md): the MANUAL move of
  // EXISTING risk between risk portfolios — the complement to routing (which
  // auto-assigns NEW fills). Three single-asset FI surfaces, initiate/accept gated
  // on the narrow `risk_transfer` capability (see {@link WORKSPACE_CAPABILITY}); the
  // audit trail stays `view` for any FI trader. Their subtitles disambiguate the
  // trio the way FI-BOOK-CONCEPTS disambiguates the "Book" family.
  { id: "risktransfer", glyph: "⇆", label: "Risk Transfer", subtitle: "Move existing risk between portfolios", assets: ["fixed_income"] },
  { id: "transferinbox", glyph: "⇱", label: "Transfer Inbox", subtitle: "Approve incoming transfers", assets: ["fixed_income"] },
  { id: "transferaudit", glyph: "❑", label: "Transfer Audit", subtitle: "Who moved what · when · at what price", assets: ["fixed_income"] },
  { id: "surface", glyph: "◷", label: "Market Data", subtitle: "Curves & vol surface", assets: CAPABILITY_ASSETS },
  // The class-parametric SCENARIO risk grid (spot×vol P&L / rates netted risk) — an
  // analytics view, NOT the routed-risk roll-up (Risk Dashboard) nor the ledger.
  { id: "risk", glyph: "⊞", label: "Risk", subtitle: "Scenario P&L / greeks", assets: CAPABILITY_ASSETS },
  // The position LEDGER — booked positions, booking, and deals (what you hold), NOT
  // the LP price composite (Agg Book) nor a risk-management bucket (Risk Portfolios).
  { id: "book", glyph: "▤", label: "Book", subtitle: "Positions · deals · P&L", assets: CAPABILITY_ASSETS },
  { id: "quoting", glyph: "⇌", label: "Quoting", subtitle: "RFQ / IOI desk inbox", assets: ["fixed_income"] },
  { id: "xva", glyph: "⊗", label: "XVA", assets: ["fx_options"] },
  { id: "excel", glyph: "▦", label: "Excel", assets: ["fx_options"] },
  // Administration / ops — admin-gated, no license concept. (Connections is venue
  // ops: its rail stays admin-gated, but its in-pane edits gate on `manage_liquidity`
  // server-side — see AggregatedBookWorkspace for the reachable manage_liquidity
  // affordance an FI-liquidity seat uses.)
  { id: "connections", glyph: "⇄", label: "Connections", assets: [] },
  { id: "admin", glyph: "⚇", label: "Admin", assets: [] },
  { id: "permissions", glyph: "⚷", label: "Permissions", assets: [] },
  // Pricing Groups is a Fixed-Income CLIENT-PRICING surface, not identity admin: it
  // moved OFF the Administration tab onto the FI tab (assets: fixed_income) and gates
  // rail visibility + structure edits on `manage_pricing·FI`, so the FI pricing desk
  // sees and edits it WITHOUT full Administer (docs/PERMISSIONS-GRANULAR-REVIEW.md §4).
  { id: "pricinggroups", glyph: "⚙", label: "Pricing Groups", subtitle: "Per-client feature pipelines", assets: ["fixed_income"], viewCap: { action: "manage_pricing", asset: "fixed_income" } },
  { id: "refdata", glyph: "❏", label: "Reference Data", assets: [] },
] as const;

/**
 * The asset class(es) a workspace serves — looked up via {@link RAIL}. Cross-asset
 * (class-parametric) rows return both classes; single-asset rows one; admin/ops
 * rows the empty list. Drives navigation gating (reachable if the identity can view
 * ANY served class) and the license three-state, replacing the retired per-domain
 * asset mapping now that the rail no longer splits by asset class.
 */
export function workspaceAssets(id: WorkspaceId): readonly CapabilityAsset[] {
  const entry = RAIL.find((r) => r.id === id);
  if (!entry) {
    throw new Error(`workspaceAssets: unknown workspace id \`${id}\` (not in RAIL)`);
  }
  return entry.assets;
}

// ---------------------------------------------------------------------------
// Navigation gating (slice 5c) — HIDE rail workspaces a signed-in user has no
// access to, exactly as the admin panes are hidden for non-admins. Slice 5b gated
// individual CONTROLS (disable + tooltip); this layer gates NAVIGATION: a user
// with no `view` on ANY of a workspace's served asset classes never sees that
// workspace. fe-fi-migration #6: with the FX/FI domain-tab split retired, gating
// is per-WORKSPACE-ASSET (a class-parametric row is reachable if the identity can
// view EITHER class it serves; its unlicensed/denied class is gated per-lens
// INSIDE the pane), not per-domain-tab. The predicates are PURE (no React, no
// transport) so the Shell + AppContext share one source of truth and the rules
// are unit-tested in isolation. UX-only: the server still enforces every RPC.
// ---------------------------------------------------------------------------

/**
 * The auth surface the nav-gating predicates need — a subset of `AuthApi`
 * (`hooks/useAuth`). `can` is the base capability test, which is PERMISSIVE when
 * signed out (returns `true`), so pre-login every domain stays visible; gating
 * only ever NARROWS a real signed-in identity.
 */
export interface NavAuth {
  /** Whether the identity is an administrator (governs the Administration tab). */
  isAdmin: boolean;
  /** Whether the identity holds `action` on `asset` (permissive when signed out). */
  can: (action: CapabilityAction, asset: CapabilityAsset) => boolean;
}

/**
 * Workspaces only an administrator may open — the members of the Administration
 * domain. Reference Data is admin-managed and lives here too: the registry is
 * cross-asset and server-resolvable by any caller (pricing/curve-building), but
 * its management UI is admin-only. This is the per-workspace backstop the Shell
 * hides and the AppContext redirect bounces.
 */
export const ADMIN_ONLY_WORKSPACES: ReadonlySet<WorkspaceId> = new Set<WorkspaceId>([
  "connections",
  "admin",
  "permissions",
  "refdata",
]);

/**
 * The capability ACTION a workspace's reachability gates on, when it is NOT the
 * default `view`. A few surfaces are write-class enough that merely viewing their
 * asset does not entitle a user to reach them — the FI Risk Transfer ticket and
 * inbox are booking-class writes (initiate / accept), so they gate on the narrow
 * `risk_transfer` capability, exactly as the server does (initiate/accept require
 * `risk_transfer`; the audit trail stays `view` for any FI trader). A workspace
 * absent from this map defaults to `view` (the base "can see this at all" gate).
 * `can` is permissive signed-out, so pre-login these rails still render.
 */
export const WORKSPACE_CAPABILITY: Partial<Record<WorkspaceId, CapabilityAction>> = {
  risktransfer: "risk_transfer",
  transferinbox: "risk_transfer",
};

/**
 * Whether a single WORKSPACE is reachable by this identity. Admin-only workspaces
 * require `isAdmin`; every trading workspace is reachable if the identity holds the
 * workspace's gating action ({@link WORKSPACE_CAPABILITY}, default `view`) on AT
 * LEAST ONE of the asset classes it serves (a class-parametric row — Ticket /
 * Market Data / Risk / Book — is reachable via EITHER FX or FI; a single-asset row
 * via that one). `view` is the right gate for ordinary "can see this at all"
 * surfaces (write controls remain individually gated by 5b); the write-class Risk
 * Transfer surfaces gate on `risk_transfer` so a booking-only trader without the
 * narrow transfer grant never reaches the ticket/inbox. The denied/unlicensed class
 * is gated per-lens INSIDE the pane. Signed out, `can` is permissive ⇒ every trading
 * workspace stays visible. Used by the rail filter, the palette/⌘N command filter,
 * and the AppContext redirect so no path can strand a user on a hidden workspace.
 */
export function workspaceAccessible(id: WorkspaceId, auth: NavAuth): boolean {
  // Per-feature visibility wins (docs/PERMISSIONS-GRANULAR-REVIEW.md §4.1): a row
  // with a `viewCap` is visible ONLY to a holder of that fine-grained capability
  // (admin holds `grant_all`; `can` is permissive signed-out). This is what hides
  // the FI management surfaces (Risk Portfolios/Routing/Dashboard, Tiering, Pricing
  // Groups) from an ordinary trader while surfacing them to the granted manager.
  const viewCap = RAIL.find((r) => r.id === id)?.viewCap;
  if (viewCap) return auth.can(viewCap.action, viewCap.asset);
  if (ADMIN_ONLY_WORKSPACES.has(id)) return auth.isAdmin;
  const action = WORKSPACE_CAPABILITY[id] ?? "view";
  return workspaceAssets(id).some((asset) => auth.can(action, asset));
}

/**
 * The first workspace (in {@link RAIL} order) this identity can reach, or `null`
 * if none — the redirect target when the active workspace's domain becomes
 * inaccessible. Excel is always reachable, so a signed-in identity always has at
 * least one accessible workspace; `null` is a defensive degenerate only.
 *
 * When `domain` is given, the search is restricted to the rows belonging to that
 * domain tab (see {@link workspaceDomains}); if that domain has no accessible row
 * it falls back to the GLOBAL first accessible workspace, so a caller can prefer
 * landing WITHIN the active domain without risking a `null` when the domain is
 * empty but other domains are reachable.
 */
export function firstAccessibleWorkspace(
  auth: NavAuth,
  domain?: Domain,
): WorkspaceId | null {
  if (domain !== undefined) {
    const inDomain = railForDomain(domain).find((r) => workspaceAccessible(r.id, auth));
    if (inDomain) return inDomain.id;
    // Domain has no reachable row — fall back to the global first accessible.
  }
  const entry = RAIL.find((r) => workspaceAccessible(r.id, auth));
  return entry ? entry.id : null;
}

// ---------------------------------------------------------------------------
// Domain layer (fe-fi-migration re-add) — a top-level product-DOMAIN tab bar
// (FX Options / Fixed Income / Administration) ABOVE the single class-parametric
// rail. Membership is DERIVED from each row's served `assets` (+ the admin-only
// set), never a redundant per-row `domain` field: a cross-asset row appears under
// BOTH trading tabs (Model A), a single-asset row under its one tab, admin/ops
// rows under the single "admin" domain. The tab only selects a LENS for the
// shared screens; the rail still applies {@link railState} per row.
// ---------------------------------------------------------------------------

/** A top-level product domain (tab). Trading domains ARE their CapabilityAsset. */
export type Domain = CapabilityAsset | "admin";

/** The top-level domain tabs, in bar order. */
export const DOMAINS: readonly { id: Domain; label: string }[] = [
  { id: "fx_options", label: "FX Options" },
  { id: "fixed_income", label: "Fixed Income" },
  { id: "admin", label: "Administration" },
] as const;

/**
 * The domain tab(s) a workspace appears under — DERIVED from its served assets.
 * Cross-asset rows appear under BOTH FX and FI (Model A); admin/ops rows (which
 * serve no asset) under the single "admin" domain.
 */
export function workspaceDomains(id: WorkspaceId): readonly Domain[] {
  if (ADMIN_ONLY_WORKSPACES.has(id)) return ["admin"];
  const assets = workspaceAssets(id);
  return assets.length > 0 ? assets : ["admin"];
}

/**
 * Whether a top-level DOMAIN tab is accessible: admin → `isAdmin`; a trading
 * domain → `view` on its class. Signed-out `can` is permissive ⇒ both trading
 * tabs render pre-login; the Administration tab stays `isAdmin`-gated (hidden
 * signed out — matches the existing admin-pane hide; no special-case).
 */
export function domainAccessible(domain: Domain, auth: NavAuth): boolean {
  if (domain === "admin") return auth.isAdmin;
  return auth.can("view", domain);
}

/**
 * The rail rows belonging to `domain`, in {@link RAIL} order (structural
 * membership only; the Shell still applies {@link railState} per row for the
 * present / gated-upsell / hidden three-state).
 */
export function railForDomain(domain: Domain): readonly (typeof RAIL)[number][] {
  return RAIL.filter((r) => workspaceDomains(r.id).includes(domain));
}

// ---------------------------------------------------------------------------
// License gating (DEC-license-gating-and-scope) — the THREE-state rail.
//
// Entitlement (`celnet-entitlements`) and commercial LICENSE are DISTINCT gates:
//   • Not ENTITLED (deny-wins capability / information barrier) ⇒ HIDDEN — a user
//     denied a capability must not even see it (security; unchanged).
//   • Not LICENSED (the firm holds no commercial license for the asset class) ⇒
//     GATED-UPSELL — PRESENT but greyed with a lock + a "license this class"
//     affordance, so the class stays DISCOVERABLE and can be licensed (per-class
//     licensing is a first-class product primitive — this supersedes "hide" for
//     commercial gating).
//   • Otherwise ⇒ PRESENT.
// License is a per-ASSET-CLASS flag (a `LicensePredicate`), config-driven and
// DEFAULTING TO ALL-LICENSED, so the rail is byte-identical to before unless a
// deployment explicitly gates a class (then, and only then, does gated-upsell
// appear). Entitlement is evaluated FIRST so an info-barrier hide can never be
// downgraded into a visible upsell. UX-only: the server still enforces every RPC.
// ---------------------------------------------------------------------------

/** The three states a rail workspace / domain tab can render in. */
export type RailState = "present" | "gated-upsell" | "hidden";

/** The affordance title shown on a license-gated (unlicensed) rail entry / tab. */
export const LICENSE_UPSELL_TITLE = "license this class";

/**
 * Whether the firm holds a commercial license for an asset class. The default
 * ({@link ALL_LICENSED}) returns `true` for every class ⇒ no gated-upsell state,
 * so existing behavior is unchanged until a class is explicitly unlicensed.
 */
export type LicensePredicate = (asset: CapabilityAsset) => boolean;

/** The default license predicate: every asset class licensed (no behavior change). */
export const ALL_LICENSED: LicensePredicate = () => true;

/** Build a license predicate that treats exactly `unlicensed` as NOT licensed. */
export function makeLicensePredicate(unlicensed: Iterable<CapabilityAsset>): LicensePredicate {
  const set = new Set(unlicensed);
  return (asset) => !set.has(asset);
}

/**
 * The three-state a WORKSPACE rail entry renders in (fe-fi-migration #6, now that
 * the rail no longer splits by asset class):
 *   • NOT reachable (entitlement-deny / non-admin on an admin pane) ⇒ HIDDEN — an
 *     information-barrier hide (wins over any upsell).
 *   • Reachable, but the firm is licensed for NONE of the entitled classes this
 *     workspace serves ⇒ GATED-UPSELL (present + lock + "license this class"). A
 *     single-asset row (Stream/Quoting) shows this when its one class is
 *     unlicensed; a cross-asset row (Ticket/Market Data/Risk/Book) only when BOTH
 *     served classes are unlicensed — while EITHER is licensed it stays PRESENT
 *     and the unlicensed lens is gated per-lens INSIDE the class-parametric pane.
 *   • Admin/ops workspaces (no served asset) have no license concept ⇒ PRESENT.
 * With the default all-licensed predicate this collapses to the two-state (present
 * iff reachable), so the rail is byte-identical unless a class is explicitly gated.
 */
export function railState(
  id: WorkspaceId,
  auth: NavAuth,
  licensed: LicensePredicate = ALL_LICENSED,
): RailState {
  if (!workspaceAccessible(id, auth)) return "hidden";
  // The classes this workspace serves that the identity is ENTITLED to view
  // (signed out, `can` is permissive ⇒ every served class). Admin/ops rows serve
  // none, so they have no license concept and stay present.
  const entitled = workspaceAssets(id).filter((asset) => auth.can("view", asset));
  if (entitled.length === 0) return "present";
  return entitled.some((asset) => licensed(asset)) ? "present" : "gated-upsell";
}

/**
 * The license predicate for THIS deployment, read from the config env
 * `VITE_CELNET_UNLICENSED` (comma/space-separated `CapabilityAsset` ids the firm
 * is NOT licensed for; unknown tokens ignored). Unset/empty ⇒ {@link ALL_LICENSED}
 * — every class licensed, so the rail is unchanged unless a deployment opts a
 * class into the gated-upsell state. Config-driven, mirroring `VITE_CELNET_*`.
 */
export function configuredLicense(): LicensePredicate {
  const raw = (import.meta.env as Record<string, string | undefined>).VITE_CELNET_UNLICENSED;
  if (!raw) return ALL_LICENSED;
  const valid = new Set<string>(CAPABILITY_ASSETS);
  const unlicensed: CapabilityAsset[] = [];
  for (const tok of raw.split(/[\s,]+/)) {
    if (valid.has(tok)) unlicensed.push(tok as CapabilityAsset);
  }
  return unlicensed.length === 0 ? ALL_LICENSED : makeLicensePredicate(unlicensed);
}

/**
 * The `⌘N` chord hint for the rail position `index` (0-based). The single-digit
 * grammar addresses the first nine views as `⌘1..⌘9`; a tenth view wraps onto
 * `⌘0` (the browser-tab convention), the last slot the digit grammar can bind.
 * A view BEYOND the ten single-digit slots (`index > 9`) has NO global chord — it
 * returns an empty chord (palette-only), so the single-source invariant holds
 * (no advertised-but-unhonoured key) rather than minting a bogus two-digit `⌘11`.
 */
export function railChord(index: number): string[] {
  if (index > 9) return [];
  return ["⌘", index === 9 ? "0" : String(index + 1)];
}

/**
 * The static command registry. Global chords + workspace jumps + the scope/action
 * commands. Workspace jumps are GENERATED from `RAIL` (one per view, `⌘1..n`), so
 * the rail and the keyboard grammar can never disagree.
 */
export const COMMAND_META: readonly CommandMeta[] = [
  // Global.
  {
    id: "palette",
    keys: ["⌘", "K"],
    group: "Global",
    label: "Open command palette",
    chord: { kind: "meta", key: "k" },
  },
  {
    id: "switch-scope",
    keys: ["⌘", "P"],
    group: "Global",
    label: "Switch underlier / scope",
    chord: { kind: "meta", key: "p" },
  },
  {
    id: "help",
    keys: ["?"],
    group: "Global",
    label: "Show the keyboard cheatsheet",
    chord: { kind: "plain", key: "?" },
  },
  // Workspaces — generated from RAIL with ⌘1..n (uncaps the old ⌘1-5).
  ...RAIL.map((r, i) => ({
    id: `ws-${r.id}`,
    keys: railChord(i),
    group: "Workspace" as const,
    label: `Go to ${r.label}`,
    // Exactly one `metaDigit` binding represents the whole ⌘1..n family; the Shell
    // resolves the pressed digit to the rail index. We tag the FIRST workspace with
    // the matcher so the single-source check sees one global digit handler, and the
    // rest carry their display chord for the cheatsheet without a duplicate matcher.
    ...(i === 0 ? { chord: { kind: "metaDigit" as const } } : {}),
  })),
  // Scope.
  {
    id: "scope-drill-down",
    keys: [],
    group: "Scope",
    label: "Drill the scope one level down",
  },
  {
    id: "scope-reset",
    keys: [],
    group: "Scope",
    label: "Reset the scope to the firm root",
  },
  // Actions (palette-only — no global chord).
  { id: "mark-surface", keys: [], group: "Action", label: "Mark surface" },
  { id: "risk-scenario", keys: [], group: "Action", label: "Open risk scenario" },
  { id: "save-view", keys: [], group: "Action", label: "Save the current view" },
  { id: "toggle-density", keys: [], group: "Action", label: "Toggle density" },
  { id: "toggle-appearance", keys: [], group: "Action", label: "Toggle light / dark" },
  { id: "toggle-contrast", keys: [], group: "Action", label: "Toggle contrast" },
] as const;

/** Look up a command's static metadata by id (undefined if unknown). */
export function commandMeta(id: string): CommandMeta | undefined {
  return COMMAND_META.find((c) => c.id === id);
}

/**
 * The cheatsheet projection: every command that advertises a chord, in registry
 * order. The `?` overlay renders EXACTLY this — it cannot drift from what the Shell
 * dispatches because both read `COMMAND_META`.
 */
export function cheatsheet(): CommandMeta[] {
  return COMMAND_META.filter((c) => c.keys.length > 0);
}

/**
 * The live app actions the runnable commands close over. A narrow surface (only
 * what a command needs) so `buildCommands` stays decoupled from the full
 * `AppState` — the Shell passes these through.
 */
export interface CommandContext {
  setWorkspace: (w: WorkspaceId) => void;
  openPalette: () => void;
  openScopeSwitcher: () => void;
  showShortcuts: () => void;
  drillScopeDown: () => void;
  resetScope: () => void;
  markSurface: () => void;
  openRiskScenario: () => void;
  saveView: () => void;
  toggleDensity: () => void;
  toggleAppearance: () => void;
  toggleContrast: () => void;
  /** Whether the scope can drill further (false at the terminal pair crumb). */
  canDrillScope: boolean;
}

/**
 * Bind the static registry to live app actions, producing the runnable
 * `Command[]` the palette renders and the Shell dispatches. The `hint` is the
 * command's display chord (so the palette shows the same legend as the
 * cheatsheet). A command with no runnable action for the current context (e.g.
 * drill-down at the terminal pair) is OMITTED, so the palette never offers a
 * dead action.
 */
export function buildCommands(ctx: CommandContext): Command[] {
  const run: Partial<Record<string, () => void>> = {
    palette: ctx.openPalette,
    "switch-scope": ctx.openScopeSwitcher,
    help: ctx.showShortcuts,
    "scope-drill-down": ctx.canDrillScope ? ctx.drillScopeDown : undefined,
    "scope-reset": ctx.resetScope,
    "mark-surface": ctx.markSurface,
    "risk-scenario": ctx.openRiskScenario,
    "save-view": ctx.saveView,
    "toggle-density": ctx.toggleDensity,
    "toggle-appearance": ctx.toggleAppearance,
    "toggle-contrast": ctx.toggleContrast,
  };
  // Workspace jumps are generated from RAIL.
  for (const r of RAIL) run[`ws-${r.id}`] = () => ctx.setWorkspace(r.id);

  const out: Command[] = [];
  for (const meta of COMMAND_META) {
    const action = run[meta.id];
    if (!action) continue; // context-unavailable command (e.g. terminal drill).
    const cmd: Command = { id: meta.id, title: meta.label, group: meta.group, run: action };
    if (meta.keys.length > 0) cmd.hint = meta.keys.join("");
    out.push(cmd);
  }
  return out;
}

/**
 * Resolve a keydown against the registry's global chords, returning the matched
 * command id (or null). Pure — the Shell wires the `KeyboardEvent` to this and
 * dispatches the resolved id, so the honoured grammar IS the registry. `meta` is
 * `metaKey || ctrlKey`; `digitCount` caps the `⌘1..n` family to the rail length.
 */
export function resolveChord(
  e: { key: string; meta: boolean },
  digitCount: number,
): { id: string; railIndex?: number } | null {
  for (const cmd of COMMAND_META) {
    const c = cmd.chord;
    if (!c) continue;
    if (c.kind === "meta" && e.meta && e.key.toLowerCase() === c.key) {
      return { id: cmd.id };
    }
    if (c.kind === "metaDigit" && e.meta && /^[0-9]$/.test(e.key)) {
      // ⌘1..⌘9 ⇒ views 0..8; ⌘0 wraps to the tenth view (index 9).
      const idx = e.key === "0" ? 9 : Number(e.key) - 1;
      if (idx < digitCount) return { id: `ws-${RAIL[idx]!.id}`, railIndex: idx };
    }
    if (c.kind === "plain" && !e.meta && e.key === c.key) {
      return { id: cmd.id };
    }
  }
  return null;
}
