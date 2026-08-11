/**
 * Panel — the workspace material primitive (GUI-DESIGN §3.3). A `bg-raised`
 * surface with a hairline top highlight and a soft 1-level shadow. Depth, not
 * chrome (principle 7): hierarchy comes from the material, not gridlines.
 *
 * Keyboard access (WCAG 2.1.1 / axe `scrollable-region-focusable`): the body is
 * an `overflow: auto` scroll container, so whenever its content GENUINELY
 * overflows it must be reachable and scrollable from the keyboard — some panel
 * bodies (the surface mesh, the vega ladder) contain no focusable content at
 * all. The measurement itself lives in {@link useScrollableRegion} (shared with
 * the `<DataTable>` scrollport so the two cannot drift): only while the body
 * actually overflows does it take `tabIndex=0` and expose itself as a `region`
 * named by the panel title. Non-overflowing panels add no tab stops.
 */

import { useId } from "react";

import { scrollableRegionProps, useScrollableRegion } from "../hooks/useScrollableRegion";
import styles from "./Panel.module.css";

export interface PanelProps {
  title?: React.ReactNode;
  glyph?: string;
  actions?: React.ReactNode;
  children: React.ReactNode;
  /** "panel" (default) | "float" (translucent overlay) | "hud" (thick). */
  material?: "panel" | "float" | "hud";
  className?: string | undefined;
  noPadding?: boolean;
}

export function Panel({
  title,
  glyph,
  actions,
  children,
  material = "panel",
  className,
  noPadding,
}: PanelProps): React.ReactElement {
  const [bodyRef, scrollable] = useScrollableRegion<HTMLDivElement>();
  const titleId = useId();

  const hasTitle = title !== undefined;
  return (
    <section
      className={[
        styles.panel,
        styles[material],
        noPadding ? styles.noPadding : "",
        className ?? "",
      ]
        .filter(Boolean)
        .join(" ")}
    >
      {hasTitle && (
        <header className={styles.header}>
          {glyph && <span className={styles.glyph}>{glyph}</span>}
          <h2 id={titleId} className={styles.title}>
            {title}
          </h2>
          {actions && <div className={styles.actions}>{actions}</div>}
        </header>
      )}
      <div
        ref={bodyRef}
        className={styles.body}
        {...scrollableRegionProps(scrollable, hasTitle ? { labelledBy: titleId } : {})}
      >
        {children}
      </div>
    </section>
  );
}
