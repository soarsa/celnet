/**
 * PricingGroupRowMenu — a small, accessible, cursor-anchored context menu for a pricing
 * group roster row. It is opened by a right-click on a row (or the row's context-menu
 * key / Shift+F10) and offers the single row-scoped "Create hedging rule" action, which
 * seeds a hedge exit-policy rule from the group and deep-links to the Hedging workspace.
 *
 * The deal-blotter equivalent ({@link ./FlowRowContextMenu}) is coupled to a
 * counterparty + an always-present "Create acceptance rule" item, so rather than bend it
 * this is a focused local menu. It shares the SAME a11y contract and CSS module: portalled
 * to `document.body` and viewport-clamped (a row inside an `overflow:auto` list still gets
 * an on-screen menu), `role="menu"` with a roving `role="menuitem"`, Escape + outside-
 * pointer dismissal, and focus returned to the opener on close.
 */
import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";

import menu from "./FlowRowContextMenu.module.css";

/** Where + for which group a row menu is open: the group's identity and a viewport point. */
export interface PricingGroupMenuTarget {
  /** The group's id — carried so the handler seeds from the exact group. */
  groupId: string;
  /** The group's display name — shown in the item sub-label. */
  name: string;
  /** Viewport x (px) to anchor the menu's left edge at (clamped inside the viewport). */
  x: number;
  /** Viewport y (px) to anchor the menu's top edge at (clamped inside the viewport). */
  y: number;
}

interface PricingGroupRowMenuProps {
  /** The open target, or `null` when closed. */
  target: PricingGroupMenuTarget | null;
  /** Close the menu (clears the parent's target). */
  onClose: () => void;
  /** Invoke "Create hedging rule" for the row's group id — seeds + deep-links to Hedging. */
  onCreateHedgingRule: (groupId: string) => void;
}

/** Viewport margin (px) the clamp keeps around the menu. */
const MARGIN = 8;

export function PricingGroupRowMenu({
  target,
  onClose,
  onCreateHedgingRule,
}: PricingGroupRowMenuProps): React.ReactElement | null {
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
  }, [open, target?.groupId, target?.x, target?.y]);

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
    const el = menuRef.current;
    if (!el) return;
    const items = Array.from(el.querySelectorAll<HTMLElement>('[role="menuitem"]'));
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

  return createPortal(
    <div
      ref={menuRef}
      role="menu"
      aria-label={`Actions for pricing group ${target.name}`}
      className={menu.menu}
      style={style}
      onKeyDown={onMenuKeyDown}
    >
      <button
        type="button"
        role="menuitem"
        className={menu.item}
        data-testid="pricing-group-create-hedging-rule"
        onClick={() => {
          onCreateHedgingRule(target.groupId);
          onClose();
        }}
      >
        <span className={menu.itemMain}>Create hedging rule</span>
        <span className={menu.itemSub}>{`New exit-policy rule scoped from ${target.name}`}</span>
      </button>
    </div>,
    document.body,
  );
}
