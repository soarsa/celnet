/**
 * Panel — the workspace material primitive (GUI-DESIGN §3.3). A `bg-raised`
 * surface with a hairline top highlight and a soft 1-level shadow. Depth, not
 * chrome (principle 7): hierarchy comes from the material, not gridlines.
 */

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
      {title !== undefined && (
        <header className={styles.header}>
          {glyph && <span className={styles.glyph}>{glyph}</span>}
          <h2 className={styles.title}>{title}</h2>
          {actions && <div className={styles.actions}>{actions}</div>}
        </header>
      )}
      <div className={styles.body}>{children}</div>
    </section>
  );
}
