/**
 * The left field palette. A searchable, grouped list of draggable field chips: a
 * trader drags a chip onto the canvas to spawn a Condition node pre-bound to that
 * field (or onto an existing node to rebind it). A distinct "Book leaf" chip drops
 * a routing destination. Drag uses the same HTML5 `dataTransfer` protocol as
 * {@link ../PricingGroupsWorkspace}: the payload is `field:<name>` or `book`.
 */
import { useMemo, useState } from "react";

import { FIELD_GROUPS, FIELD_REGISTRY, type FieldGroup, type FieldSpec } from "../../lib/routeFields";
import styles from "./RiskRoutingWorkspace.module.css";

/** The drag payload written to `text/plain` — parsed by the canvas drop handler. */
export type DragPayload = { kind: "field"; field: FieldSpec["field"] } | { kind: "book" };

/** Encode a palette drag payload. */
export function encodeDrag(p: DragPayload): string {
  return p.kind === "field" ? `field:${p.field}` : "book";
}

/** Decode a drop payload (returns null for anything unrecognised). */
export function decodeDrag(raw: string): DragPayload | null {
  if (raw === "book") return { kind: "book" };
  if (raw.startsWith("field:")) {
    const field = raw.slice("field:".length);
    if (FIELD_REGISTRY.some((s) => s.field === field)) {
      return { kind: "field", field: field as FieldSpec["field"] };
    }
  }
  return null;
}

interface FieldPaletteProps {
  /** Whether the palette is interactive (admins) or a static reference (read-only). */
  readOnly: boolean;
  /** Whether to show the "Book leaf" destination chip (default true). The rule
   * editor hides it — its destination is a dedicated desk→book picker, not a chip. */
  showBookChip?: boolean;
}

const KIND_TAG: Record<FieldSpec["kind"], string> = {
  enum: "enum",
  numeric: "num",
  string: "text",
};

export function FieldPalette({ readOnly, showBookChip = true }: FieldPaletteProps): React.ReactElement {
  const [query, setQuery] = useState("");

  const grouped = useMemo(() => {
    const q = query.trim().toLowerCase();
    const match = (s: FieldSpec): boolean =>
      q.length === 0 || s.label.toLowerCase().includes(q) || s.field.includes(q);
    const out = new Map<FieldGroup, FieldSpec[]>();
    for (const g of FIELD_GROUPS) out.set(g, []);
    for (const s of FIELD_REGISTRY) if (match(s)) out.get(s.group)?.push(s);
    return out;
  }, [query]);

  const onDragStart =
    (payload: DragPayload) =>
    (e: React.DragEvent<HTMLDivElement>): void => {
      e.dataTransfer.setData("text/plain", encodeDrag(payload));
      e.dataTransfer.effectAllowed = "copy";
    };

  return (
    <aside className={styles.palette} aria-label="Field palette">
      <div className={styles.paletteHead}>
        <h2 className={styles.paletteTitle}>Fields</h2>
        <input
          className={styles.paletteSearch}
          type="search"
          placeholder="Search fields…"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          aria-label="Search fields"
        />
      </div>

      {showBookChip && (
        <div
          className={`${styles.chip} ${styles.chipBook}`}
          draggable={!readOnly}
          onDragStart={onDragStart({ kind: "book" })}
          aria-label="Drag to add a book destination"
          data-testid="palette-book-chip"
        >
          <span className={styles.chipGlyph} aria-hidden>
            ❦
          </span>
          <span className={styles.chipMain}>
            <span className={styles.chipLabel}>Book leaf</span>
            <span className={styles.chipHint}>A risk-book destination.</span>
          </span>
        </div>
      )}

      {FIELD_GROUPS.map((group) => {
        const items = grouped.get(group) ?? [];
        if (items.length === 0) return null;
        return (
          <section key={group} className={styles.paletteGroup}>
            <h3 className={styles.paletteGroupLabel}>{group}</h3>
            {items.map((s) => (
              <div
                key={s.field}
                className={`${styles.chip} ${styles.chipField}`}
                draggable={!readOnly}
                onDragStart={onDragStart({ kind: "field", field: s.field })}
                aria-label={`Drag ${s.label} onto the canvas`}
                data-testid={`palette-field-${s.field}`}
              >
                <span className={`${styles.chipKind} ${styles[`kind_${s.kind}`]}`}>
                  {KIND_TAG[s.kind]}
                </span>
                <span className={styles.chipMain}>
                  <span className={styles.chipLabel}>{s.label}</span>
                  <span className={styles.chipHint}>{s.hint}</span>
                </span>
              </div>
            ))}
          </section>
        );
      })}

      {readOnly && (
        <p className={styles.paletteReadonly}>Read-only — sign in as an admin to edit routing.</p>
      )}
    </aside>
  );
}
