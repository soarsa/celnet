/**
 * FlowRowContextMenu — a small, accessible, cursor-anchored context menu for a live-flow
 * table row (the Deals / Quotes blotters). It is opened by a right-click on a row (or a
 * row's context-menu key, or a row's ⋯ kebab) and offers row-scoped actions — today the
 * single "Create acceptance rule", which spawns an acceptance rule pre-populated with the
 * row's counterparty.
 *
 * Ownership split: the PARENT owns the open state ({@link FlowRowMenuTarget} — the
 * counterparty + the viewport point to anchor at) so a big blotter renders exactly ONE
 * menu, not one hook per row. This component owns only the presentation + the menu
 * a11y contract: portalled to `document.body` and viewport-clamped (so a row inside an
 * `overflow:auto` grid still gets an on-screen menu), `role="menu"` with roving
 * `role="menuitem"` focus, arrow / Home / End navigation, Escape + outside-pointer
 * dismissal, and focus returned to the opener on close.
 */
import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";

import styles from "./FlowRowContextMenu.module.css";

/** Where + for whom a row menu is open: the row's counterparty and a viewport point. */
export interface FlowRowMenuTarget {
  /** The row's counterparty — the value the seeded rule matches on. */
  counterparty: string;
  /** Viewport x (px) to anchor the menu's left edge at (clamped inside the viewport). */
  x: number;
  /** Viewport y (px) to anchor the menu's top edge at (clamped inside the viewport). */
  y: number;
}

interface FlowRowContextMenuProps {
  /** The open target, or `null` when closed. */
  target: FlowRowMenuTarget | null;
  /** Close the menu (clears the parent's target). */
  onClose: () => void;
  /** Invoke "Create acceptance rule" for the row's counterparty. */
  onCreateAcceptanceRule: (counterparty: string) => void;
  /**
   * Whether the viewer can author acceptance policy (`manage_acceptance`). Only tunes the
   * item's sub-label ("ask an admin to save" when read-only) — the action is always
   * offered so the surface is discoverable; the builder itself lands the seed read-only.
   */
  canManageAcceptance: boolean;
}

/** Viewport margin (px) the clamp keeps around the menu. */
const MARGIN = 8;

export function FlowRowContextMenu({
  target,
  onClose,
  onCreateAcceptanceRule,
  canManageAcceptance,
}: FlowRowContextMenuProps): React.ReactElement | null {
  const menuRef = useRef<HTMLDivElement | null>(null);
  const openerRef = useRef<HTMLElement | null>(null);
  const [style, setStyle] = useState<React.CSSProperties>({
    position: "fixed",
    top: 0,
    left: 0,
    visibility: "hidden",
  });

  const open = target !== null;

  // Remember the opener + focus the first item on open, so a keyboard user lands inside
  // the menu and returns to where they were on close.
  useEffect(() => {
    if (!open) return;
    openerRef.current = document.activeElement as HTMLElement | null;
    const first = menuRef.current?.querySelector<HTMLElement>('[role="menuitem"]');
    first?.focus();
  }, [open, target?.counterparty, target?.x, target?.y]);

  // Measure then clamp the menu inside the viewport (position: fixed at the point).
  useLayoutEffect(() => {
    if (!open || target === null) return;
    const el = menuRef.current;
    if (!el) return;
    const rect = el.getBoundingClientRect();
    const vw = window.innerWidth;
    const vh = window.innerHeight;
    const left = Math.max(MARGIN, Math.min(target.x, vw - rect.width - MARGIN));
    const top = Math.max(MARGIN, Math.min(target.y, vh - rect.height - MARGIN));
    setStyle({ position: "fixed", top, left, visibility: "visible" });
  }, [open, target?.x, target?.y]);

  const close = useCallback((): void => {
    onClose();
    openerRef.current?.focus?.();
  }, [onClose]);

  // Dismissal: Escape (returns focus), and a pointer-down outside the menu.
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent): void => {
      if (e.key === "Escape") {
        e.stopPropagation();
        close();
      }
    };
    const onDown = (e: PointerEvent): void => {
      const t = e.target as Node | null;
      if (t && menuRef.current?.contains(t)) return;
      onClose();
    };
    document.addEventListener("keydown", onKey, true);
    document.addEventListener("pointerdown", onDown, true);
    return () => {
      document.removeEventListener("keydown", onKey, true);
      document.removeEventListener("pointerdown", onDown, true);
    };
  }, [open, close, onClose]);

  // Roving focus among the menu items (Arrow / Home / End).
  const onMenuKeyDown = useCallback((e: React.KeyboardEvent<HTMLDivElement>): void => {
    const menu = menuRef.current;
    if (!menu) return;
    const items = Array.from(menu.querySelectorAll<HTMLElement>('[role="menuitem"]'));
    if (items.length === 0) return;
    const idx = items.indexOf(document.activeElement as HTMLElement);
    let next = -1;
    if (e.key === "ArrowDown") next = (idx + 1 + items.length) % items.length;
    else if (e.key === "ArrowUp") next = (idx - 1 + items.length) % items.length;
    else if (e.key === "Home") next = 0;
    else if (e.key === "End") next = items.length - 1;
    if (next >= 0) {
      e.preventDefault();
      items[next]?.focus();
    }
  }, []);

  if (!open || target === null) return null;

  const cp = target.counterparty;
  return createPortal(
    <div
      ref={menuRef}
      role="menu"
      aria-label={`Actions for ${cp}`}
      className={styles.menu}
      style={style}
      onKeyDown={onMenuKeyDown}
    >
      <button
        type="button"
        role="menuitem"
        className={styles.item}
        data-testid="flow-create-acceptance-rule"
        onClick={() => {
          onCreateAcceptanceRule(cp);
          onClose();
        }}
      >
        <span className={styles.itemMain}>Create acceptance rule</span>
        <span className={styles.itemSub}>
          {canManageAcceptance
            ? `Counterparty = ${cp}`
            : `${cp} · view only — ask an admin to save`}
        </span>
      </button>
    </div>,
    document.body,
  );
}
