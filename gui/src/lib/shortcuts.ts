/**
 * shortcuts.ts — the keyboard-cheatsheet PROJECTION of the single command registry.
 *
 * GW1 made `lib/commands.ts` the ONE source-of-truth for the keyboard grammar (the
 * chords the Shell dispatches AND advertises). This module is now a thin projection
 * OF that registry for the discoverable `?` cheatsheet (`ShortcutsOverlay`): it maps
 * the registry's chord-bearing commands into the `Shortcut` rows the overlay renders.
 *
 * Because the cheatsheet is GENERATED from the registry, the advertised bindings can
 * never drift from the honoured ones — there is no second list to keep in sync. The
 * overlay-internal grammar (↑↓ / ↵ / Esc inside the palette & scope switcher) is
 * documented here too, as those keys are component-local (not global chords).
 */

import { cheatsheet, type CommandGroup } from "./commands";

/** A logical grouping of related shortcuts in the cheatsheet. */
export type ShortcutGroup = CommandGroup | "Overlays";

/** One keyboard binding row: the chord tokens, its action label, and its group. */
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
 * The overlay-LOCAL grammar — keys honoured INSIDE the command palette and the
 * scope switcher (not global Shell chords, so they aren't in the command registry).
 * Documented so the cheatsheet is complete; provenance is `CommandPalette.tsx` +
 * `ScopeSwitcher.tsx`.
 */
const OVERLAY_SHORTCUTS: readonly Shortcut[] = [
  { id: "ov-move", keys: ["↑", "↓"], label: "Move the highlight", group: "Overlays" },
  { id: "ov-select", keys: ["↩"], label: "Activate the highlight", group: "Overlays" },
  { id: "ov-fav", keys: ["⌘", "D"], label: "Favourite the pair (switcher)", group: "Overlays" },
  { id: "ov-close", keys: ["Esc"], label: "Close the overlay", group: "Overlays" },
];

/**
 * The complete cheatsheet binding list: the global chords PROJECTED from the
 * command registry, followed by the overlay-local grammar. In display order.
 */
export const SHORTCUTS: readonly Shortcut[] = [
  ...cheatsheet().map(
    (c): Shortcut => ({ id: c.id, keys: [...c.keys], label: c.label, group: c.group }),
  ),
  ...OVERLAY_SHORTCUTS,
];

/** The cheatsheet section order (drives the overlay grouping). */
export const SHORTCUT_GROUPS: readonly ShortcutGroup[] = [
  "Global",
  "Workspace",
  "Scope",
  "Action",
  "Overlays",
];

/** Group the flat binding list into ordered sections (empty sections dropped). */
export function groupedShortcuts(): { group: ShortcutGroup; items: Shortcut[] }[] {
  return SHORTCUT_GROUPS.map((group) => ({
    group,
    items: SHORTCUTS.filter((s) => s.group === group),
  })).filter((section) => section.items.length > 0);
}
