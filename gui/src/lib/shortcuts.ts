/**
 * shortcuts.ts — the SINGLE source-of-truth for the GUI's keyboard grammar.
 *
 * The product is keyboard-first (GUI-DESIGN principle 6): the command palette,
 * workspace jumps, pair browse, and the in-context overlay/dialog grammar all
 * live here so the discoverable cheatsheet (`ShortcutsOverlay`) and the live
 * key handler (`Shell`) read the EXACT same map — no drift between what the app
 * does and what it advertises.
 *
 * Each binding carries the key tokens (rendered as <kbd>s), a human label, and a
 * group for sectioning the cheatsheet. The `keys` are display tokens — `⌘` is the
 * meta/ctrl chord the Shell binds (metaKey || ctrlKey), shown as `⌘` on every
 * platform for a single canonical legend (matching the rail/title-bar hints).
 *
 * Provenance of each binding (so the cheatsheet never overclaims):
 *   • ⌘K / ⌘P / ⌘B / ?  — `Shell.tsx` global key handler.
 *   • ⌘1–5              — `Shell.tsx` (RAIL workspace jumps).
 *   • ↩ / ⌘↩            — `TicketWorkspace.tsx` (request quote / accept side).
 *   • ↑↓ / ↩ / Esc / ⌘D — `CommandPalette.tsx` + `UniverseNavigator.tsx` grammar.
 */

/** A logical grouping of related shortcuts in the cheatsheet. */
export type ShortcutGroup = "Global" | "Workspaces" | "Ticket" | "Overlays";

/** One keyboard binding: the chord tokens, what it does, and its group. */
export interface Shortcut {
  /** Stable id (test/aria keying). */
  id: string;
  /** Ordered key tokens forming the chord (e.g. `["⌘", "K"]`). */
  keys: string[];
  /** Human-readable action label. */
  label: string;
  /** Section the binding belongs to. */
  group: ShortcutGroup;
}

/**
 * The complete binding grammar, in cheatsheet display order. This mirrors the
 * handlers in `Shell.tsx` (⌘K / ⌘1–5 / ⌘P / ⌘B / ?), the overlay/dialog grammar
 * shared by the CommandPalette + UniverseNavigator (↑↓ / ↵ / Esc / ⌘D), and the
 * ticket's price/confirm chord (⌘↩ / Esc) — every advertised binding is one the
 * product actually honours.
 */
export const SHORTCUTS: readonly Shortcut[] = [
  // Global.
  { id: "palette", keys: ["⌘", "K"], label: "Open command palette", group: "Global" },
  { id: "search", keys: ["⌘", "P"], label: "Search / jump (palette)", group: "Global" },
  { id: "browse-pairs", keys: ["⌘", "B"], label: "Browse the pair universe", group: "Global" },
  { id: "help", keys: ["?"], label: "Show this keyboard cheatsheet", group: "Global" },
  // Workspaces.
  { id: "ws-ticket", keys: ["⌘", "1"], label: "Go to Ticket", group: "Workspaces" },
  { id: "ws-stream", keys: ["⌘", "2"], label: "Go to Stream", group: "Workspaces" },
  { id: "ws-surface", keys: ["⌘", "3"], label: "Go to Surface", group: "Workspaces" },
  { id: "ws-risk", keys: ["⌘", "4"], label: "Go to Risk", group: "Workspaces" },
  { id: "ws-book", keys: ["⌘", "5"], label: "Go to Book", group: "Workspaces" },
  // Ticket.
  { id: "ticket-request", keys: ["↩"], label: "Request a quote", group: "Ticket" },
  { id: "ticket-accept", keys: ["⌘", "↩"], label: "Accept the offered side", group: "Ticket" },
  // Overlays (palette · pair navigator).
  { id: "ov-move", keys: ["↑", "↓"], label: "Move the highlight", group: "Overlays" },
  { id: "ov-select", keys: ["↩"], label: "Activate the highlight", group: "Overlays" },
  { id: "ov-fav", keys: ["⌘", "D"], label: "Favourite the pair (navigator)", group: "Overlays" },
  { id: "ov-close", keys: ["Esc"], label: "Close the overlay", group: "Overlays" },
];

/** The cheatsheet section order (drives the overlay grouping). */
export const SHORTCUT_GROUPS: readonly ShortcutGroup[] = [
  "Global",
  "Workspaces",
  "Ticket",
  "Overlays",
];

/** Group the flat binding list into ordered sections (empty sections dropped). */
export function groupedShortcuts(): { group: ShortcutGroup; items: Shortcut[] }[] {
  return SHORTCUT_GROUPS.map((group) => ({
    group,
    items: SHORTCUTS.filter((s) => s.group === group),
  })).filter((section) => section.items.length > 0);
}
