/**
 * Keyboard-shortcut cheatsheet coverage (PC-SHORTCUTS).
 *
 * Two layers, both over the REAL modules (no mocks of the components):
 *   1. The `src/lib/shortcuts.ts` source-of-truth — grouping, uniqueness, and the
 *      exact bindings the Shell/overlays honour (so the cheatsheet can't drift).
 *   2. The `ShortcutsOverlay` component — rendered standalone, it lists EVERY
 *      documented binding from that same source, is a labelled modal dialog, and
 *      closes on Escape / the close button.
 *
 * The end-to-end `?` → overlay path through the real Shell is exercised in
 * `shellShortcuts.test.tsx`.
 */
import { act } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, within } from "@testing-library/react";

import {
  SHORTCUTS,
  SHORTCUT_GROUPS,
  groupedShortcuts,
} from "../src/lib/shortcuts";
import { ShortcutsOverlay } from "../src/components/ShortcutsOverlay";

afterEach(() => {
  document.body.innerHTML = "";
});

describe("shortcuts source-of-truth", () => {
  it("every binding has a unique id, a non-empty label, and at least one key", () => {
    const ids = new Set<string>();
    for (const s of SHORTCUTS) {
      expect(s.label.length).toBeGreaterThan(0);
      expect(s.keys.length).toBeGreaterThan(0);
      expect(s.keys.every((k) => k.length > 0)).toBe(true);
      expect(ids.has(s.id)).toBe(false);
      ids.add(s.id);
    }
  });

  it("every binding's group is one of the declared section groups", () => {
    for (const s of SHORTCUTS) {
      expect(SHORTCUT_GROUPS).toContain(s.group);
    }
  });

  it("groupedShortcuts preserves the section order and covers every binding", () => {
    const sections = groupedShortcuts();
    // Section order is a subsequence of the declared group order.
    const order = sections.map((s) => s.group);
    expect(order).toEqual(SHORTCUT_GROUPS.filter((g) => order.includes(g)));
    // No binding is dropped or duplicated by the grouping.
    const flat = sections.flatMap((s) => s.items.map((i) => i.id));
    expect(flat.sort()).toEqual([...SHORTCUTS].map((s) => s.id).sort());
  });

  it("documents the load-bearing global bindings the Shell actually honours", () => {
    const byKeys = (keys: string[]): boolean =>
      SHORTCUTS.some((s) => s.keys.join("") === keys.join(""));
    expect(byKeys(["⌘", "K"])).toBe(true); // command palette
    expect(byKeys(["⌘", "P"])).toBe(true); // scope / underlier switcher (GW1)
    expect(byKeys(["?"])).toBe(true); // this cheatsheet
    expect(byKeys(["⌘", "1"])).toBe(true); // workspace jump
    expect(byKeys(["⌘", "5"])).toBe(true); // workspace jump
    // GW1 absorbed the pair navigator into the ONE scope control: the dead ⌘B
    // pair-browse chord is GONE (no advertised-but-unhandled key).
    expect(byKeys(["⌘", "B"])).toBe(false);
  });
});

describe("ShortcutsOverlay component", () => {
  it("renders nothing when closed", () => {
    const { container } = render(<ShortcutsOverlay open={false} onClose={() => {}} />);
    expect(container.firstChild).toBeNull();
  });

  it("when open, is a labelled modal dialog listing every documented binding", () => {
    render(<ShortcutsOverlay open onClose={() => {}} />);
    const dialog = screen.getByRole("dialog", { name: "Keyboard shortcuts" });
    expect(dialog.getAttribute("aria-modal")).toBe("true");

    // Every binding's action label is present, and its key tokens render as <kbd>.
    for (const s of SHORTCUTS) {
      const action = within(dialog).getAllByText(s.label)[0];
      expect(action).toBeTruthy();
    }
    // The chord tokens render as <kbd> elements (at least one per binding shown).
    const kbds = dialog.querySelectorAll("kbd");
    expect(kbds.length).toBeGreaterThanOrEqual(SHORTCUTS.length);
  });

  it("closes on Escape", () => {
    const onClose = vi.fn();
    render(<ShortcutsOverlay open onClose={onClose} />);
    const dialog = screen.getByRole("dialog", { name: "Keyboard shortcuts" });
    act(() => {
      fireEvent.keyDown(dialog, { key: "Escape" });
    });
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("closes via the close button", () => {
    const onClose = vi.fn();
    render(<ShortcutsOverlay open onClose={onClose} />);
    fireEvent.click(screen.getByRole("button", { name: "close keyboard shortcuts" }));
    expect(onClose).toHaveBeenCalledTimes(1);
  });
});
