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
  { id: "excel", glyph: "▦", label: "Excel", domain: "administration" },
] as const;

/** The product domain (tab) a workspace belongs to — looked up via {@link RAIL}. */
export function domainOf(id: WorkspaceId): Domain {
  const entry = RAIL.find((r) => r.id === id);
  if (!entry) {
    throw new Error(`domainOf: unknown workspace id \`${id}\` (not in RAIL)`);
  }
  return entry.domain;
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
