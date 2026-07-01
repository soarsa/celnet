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

/** Workspace ids the rail exposes (kept in sync with `AppContext.WorkspaceId`). */
export type WorkspaceId =
  | "ticket"
  | "rates"
  | "curve"
  | "ratesrisk"
  | "quoting"
  | "deals"
  | "ratesbook"
  | "stream"
  | "surface"
  | "risk"
  | "book"
  | "connections"
  | "admin"
  | "permissions"
  | "excel";

/**
 * A top-level product domain — the tab a workspace lives under. The Shell renders
 * one tab per domain (Administration shown only to admins) and switching a tab
 * jumps to that domain's last-active (or first) workspace. Every RAIL entry
 * declares exactly one domain, so the three tabs partition the rail.
 */
export type Domain = "fx-options" | "fixed-income" | "administration";

/** The top-tab bar order: FX Options, then Fixed Income, then Administration. */
export const DOMAINS: readonly { id: Domain; label: string }[] = [
  { id: "fx-options", label: "FX Options" },
  { id: "fixed-income", label: "Fixed Income" },
  { id: "administration", label: "Administration" },
] as const;

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
 * The rail workspaces, in order. The data-driven rail (replacing the hard-coded
 * `RAIL`/`⌘1-5`): the `⌘N` hint and the `metaDigit` chord are DERIVED from this
 * order, so adding a view needs only a row here — its `⌘N` lights up for free
 * (`⌘1..⌘9`, then `⌘0` for a tenth; uncapping the old `⌘1-5`). Glyph fix: Book is `▤` (a ledger), freeing `Σ` for
 * sum/vega-ladder use exclusively (one glyph, one meaning).
 */
export const RAIL: readonly {
  id: WorkspaceId;
  glyph: string;
  label: string;
  /** The top-level product domain (tab) this workspace lives under. */
  domain: Domain;
}[] = [
  // FX Options.
  { id: "ticket", glyph: "⌁", label: "Ticket", domain: "fx-options" },
  { id: "stream", glyph: "≋", label: "Stream", domain: "fx-options" },
  { id: "surface", glyph: "◷", label: "Surface", domain: "fx-options" },
  { id: "risk", glyph: "⊞", label: "Risk", domain: "fx-options" },
  { id: "excel", glyph: "▦", label: "Excel", domain: "fx-options" },
  // Fixed Income.
  { id: "rates", glyph: "≣", label: "Rates", domain: "fixed-income" },
  { id: "curve", glyph: "∿", label: "Curve", domain: "fixed-income" },
  { id: "ratesrisk", glyph: "⊟", label: "Rates Risk", domain: "fixed-income" },
  { id: "quoting", glyph: "⇌", label: "Quoting", domain: "fixed-income" },
  { id: "deals", glyph: "✓", label: "Deals", domain: "fixed-income" },
  { id: "ratesbook", glyph: "▥", label: "Rates Book", domain: "fixed-income" },
  { id: "book", glyph: "▤", label: "Book", domain: "fixed-income" },
  // Administration.
  { id: "connections", glyph: "⇄", label: "Connections", domain: "administration" },
  { id: "admin", glyph: "⚇", label: "Admin", domain: "administration" },
  { id: "permissions", glyph: "⚷", label: "Permissions", domain: "administration" },
] as const;

/** The product domain (tab) a workspace belongs to — looked up via {@link RAIL}. */
export function domainOf(id: WorkspaceId): Domain {
  const entry = RAIL.find((r) => r.id === id);
  if (!entry) {
    throw new Error(`domainOf: unknown workspace id \`${id}\` (not in RAIL)`);
  }
  return entry.domain;
}

// ---------------------------------------------------------------------------
// Navigation gating (slice 5c) — HIDE whole domain tabs/workspaces a signed-in
// user has no access to, exactly as the Administration tab is hidden for non-
// admins. Slice 5b gated individual CONTROLS (disable + tooltip); this layer
// gates NAVIGATION: a user with no `view` on an asset class never sees that
// domain's tab or its rail workspaces. The predicates are PURE (no React, no
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
 * Workspaces only an administrator may open — the admin-only members of the
 * Administration domain. (Excel also lives under Administration but is NOT admin-
 * only, so it is deliberately absent.) This is the per-workspace backstop the
 * Shell hides and the AppContext redirect bounces.
 */
export const ADMIN_ONLY_WORKSPACES: ReadonlySet<WorkspaceId> = new Set<WorkspaceId>([
  "connections",
  "admin",
  "permissions",
]);

/**
 * Whether a top-level DOMAIN tab is accessible to this identity. The base read
 * capability `view` on the domain's asset class is the right gate for "can see
 * this asset class at all" (write controls remain individually gated by 5b):
 *   - `fx-options`     → `view` on `fx_options`
 *   - `fixed-income`   → `view` on `fixed_income`
 *   - `administration` → `isAdmin` (unchanged)
 * Signed out, `can` is permissive ⇒ both asset domains stay visible.
 */
export function domainAccessible(domain: Domain, auth: NavAuth): boolean {
  switch (domain) {
    case "fx-options":
      return auth.can("view", "fx_options");
    case "fixed-income":
      return auth.can("view", "fixed_income");
    case "administration":
      return auth.isAdmin;
  }
}

/**
 * Whether a single WORKSPACE is reachable by this identity. Admin-only
 * workspaces require `isAdmin`; every other workspace follows its domain's
 * accessibility (FX/FI gate on `view`, Administration on `isAdmin`). Used by the
 * rail filter, the palette/⌘N command filter, and the AppContext redirect so no
 * path can strand a user on a hidden workspace.
 */
export function workspaceAccessible(id: WorkspaceId, auth: NavAuth): boolean {
  if (ADMIN_ONLY_WORKSPACES.has(id)) return auth.isAdmin;
  return domainAccessible(domainOf(id), auth);
}

/**
 * The first workspace (in {@link RAIL} order) this identity can reach, or `null`
 * if none — the redirect target when the active workspace's domain becomes
 * inaccessible. Excel is always reachable, so a signed-in identity always has at
 * least one accessible workspace; `null` is a defensive degenerate only.
 */
export function firstAccessibleWorkspace(auth: NavAuth): WorkspaceId | null {
  const entry = RAIL.find((r) => workspaceAccessible(r.id, auth));
  return entry ? entry.id : null;
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
 * The asset class a domain licenses under, or `null` when the domain has no
 * commercial-license concept (Administration is admin-gated, never licensed).
 */
export function assetOfDomain(domain: Domain): CapabilityAsset | null {
  switch (domain) {
    case "fx-options":
      return "fx_options";
    case "fixed-income":
      return "fixed_income";
    case "administration":
      return null;
  }
}

/** The asset class a workspace licenses under (via its domain), or `null`. */
export function workspaceAsset(id: WorkspaceId): CapabilityAsset | null {
  return assetOfDomain(domainOf(id));
}

/**
 * The three-state a DOMAIN tab renders in: entitlement-deny ⇒ hidden;
 * entitled-but-unlicensed ⇒ gated-upsell; else present. With the default
 * all-licensed predicate this collapses to the legacy two-state (present iff
 * {@link domainAccessible}).
 */
export function domainRailState(
  domain: Domain,
  auth: NavAuth,
  licensed: LicensePredicate = ALL_LICENSED,
): RailState {
  if (!domainAccessible(domain, auth)) return "hidden";
  const asset = assetOfDomain(domain);
  if (asset !== null && !licensed(asset)) return "gated-upsell";
  return "present";
}

/**
 * The three-state a WORKSPACE rail entry renders in — the per-workspace twin of
 * {@link domainRailState}: entitlement-deny ⇒ hidden; entitled-but-unlicensed ⇒
 * gated-upsell; else present. With the default all-licensed predicate this
 * collapses to the legacy two-state (present iff {@link workspaceAccessible}).
 */
export function railState(
  id: WorkspaceId,
  auth: NavAuth,
  licensed: LicensePredicate = ALL_LICENSED,
): RailState {
  if (!workspaceAccessible(id, auth)) return "hidden";
  const asset = workspaceAsset(id);
  if (asset !== null && !licensed(asset)) return "gated-upsell";
  return "present";
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
