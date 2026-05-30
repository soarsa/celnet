/**
 * PriceTile — a tabular numeric that flashes once on value change and decays
 * (GUI-DESIGN §3.4: the single most important motion in the app). The digits
 * never move (tabular nums); only the background wash decays. Tick direction is
 * carried by both color AND an optional glyph (color-independence, §7). Honors
 * prefers-reduced-motion: the flash becomes a static directional tint.
 *
 * Direction is detected from a persistent ref (survives any DOM change) and the
 * flash is replayed by restarting the CSS animation on a stable element — never
 * by a key remount, which previously coupled (and so could corrupt) the two.
 */

import { useEffect, useLayoutEffect, useRef, useState } from "react";
import styles from "./PriceTile.module.css";

export interface PriceTileProps {
  value: number;
  format: (v: number) => string;
  /** "bid" | "offer" | "neutral" — side tint applied to the resting text. */
  side?: "bid" | "offer" | "neutral";
  /** Render the ▲/▼ direction glyph alongside the value. */
  showGlyph?: boolean;
  size?: "display" | "headline" | "callout";
  ariaLabel?: string;
}

type Dir = "up" | "down" | "none";

export function PriceTile({
  value,
  format,
  side = "neutral",
  showGlyph = false,
  size = "callout",
  ariaLabel,
}: PriceTileProps): React.ReactElement {
  // `prev` is a ref on the (stable) component instance — it is NOT reset by any
  // DOM remount, so tick direction is computed reliably across every update.
  // The flash animation is restarted IMPERATIVELY (forced reflow) on a stable
  // element rather than by remounting via `key`: a keyed remount swaps the DOM
  // node and, more importantly, made direction detection brittle and caused a
  // redundant render. Here direction (state, for the glyph) and the flash
  // (an animation restart on the same element) are decoupled.
  const prev = useRef<number>(value);
  const [dir, setDir] = useState<Dir>("none");
  const elRef = useRef<HTMLSpanElement>(null);

  useLayoutEffect(() => {
    const before = prev.current;
    prev.current = value;
    if (value === before) return;
    const nextDir: Dir = value > before ? "up" : "down";
    setDir(nextDir);

    const el = elRef.current;
    if (!el) return;
    // Restart the CSS flash: clear both animation classes, force a reflow so the
    // browser registers the removal, then re-add the directional one. This
    // replays the decay on every tick — including consecutive ticks in the SAME
    // direction, which a class that only changes on direction change would never
    // re-trigger. (`noUncheckedIndexedAccess` types module classes as possibly
    // undefined; the names are always present, so we filter defensively.)
    const up = styles.flashUp;
    const dn = styles.flashDn;
    const present = [up, dn].filter((c): c is string => typeof c === "string");
    if (present.length > 0) el.classList.remove(...present);
    // Reading offsetWidth forces a synchronous reflow (animation-restart idiom).
    void el.offsetWidth;
    const flashClass = nextDir === "up" ? up : dn;
    if (flashClass) el.classList.add(flashClass);
  }, [value]);

  // Reduced-motion: a static tint via data-dir (see CSS) rather than animation.
  useEffect(() => {
    const el = elRef.current;
    if (!el) return;
    el.dataset.dir = dir;
  }, [dir]);

  const cls = [
    styles.tile,
    styles[size],
    side === "bid" ? styles.bid : side === "offer" ? styles.offer : "",
  ]
    .filter(Boolean)
    .join(" ");

  return (
    <span ref={elRef} className={`num ${cls}`} aria-label={ariaLabel} data-dir={dir}>
      {showGlyph && dir !== "none" && (
        <span className={styles.glyph} aria-hidden>
          {dir === "up" ? "▲" : "▼"}
        </span>
      )}
      {format(value)}
    </span>
  );
}
