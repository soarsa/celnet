/**
 * The acceptance lift-field palette — the exact analogue of the hedge
 * {@link ../hedging/HedgeFieldPalette} / risk-routing FieldPalette, listing the
 * {@link AcceptanceField} vocabulary instead. A trader drags a chip into the rule
 * editor's conditions area to add a `field op value` leg. Drag uses the same HTML5
 * `dataTransfer` protocol; there is no decision chip (the acceptance leaf is a
 * dedicated decision editor, not a dragged chip).
 */
import { useMemo, useState } from "react";

import {
  ACCEPTANCE_FIELD_GROUPS,
  ACCEPTANCE_FIELD_REGISTRY,
  type AcceptanceFieldGroup,
  type AcceptanceFieldSpec,
} from "../../lib/acceptanceFields";
import rr from "../riskrouting/RiskRoutingWorkspace.module.css";

/** The drag payload written to `text/plain` (parsed by the drop handler). */
export function encodeAcceptanceDrag(field: AcceptanceFieldSpec["field"]): string {
  return `afield:${field}`;
}

/** Decode an acceptance field drop payload (null for anything unrecognised). */
export function decodeAcceptanceDrag(raw: string): AcceptanceFieldSpec["field"] | null {
  if (!raw.startsWith("afield:")) return null;
  const field = raw.slice("afield:".length);
  return ACCEPTANCE_FIELD_REGISTRY.some((s) => s.field === field)
    ? (field as AcceptanceFieldSpec["field"])
    : null;
}

const KIND_TAG: Record<AcceptanceFieldSpec["kind"], string> = {
  enum: "enum",
  numeric: "num",
  string: "text",
};

interface AcceptanceFieldPaletteProps {
  readOnly: boolean;
}

export function AcceptanceFieldPalette({
  readOnly,
}: AcceptanceFieldPaletteProps): React.ReactElement {
  const [query, setQuery] = useState("");

  const grouped = useMemo(() => {
    const q = query.trim().toLowerCase();
    const match = (s: AcceptanceFieldSpec): boolean =>
      q.length === 0 || s.label.toLowerCase().includes(q) || s.field.includes(q);
    const out = new Map<AcceptanceFieldGroup, AcceptanceFieldSpec[]>();
    for (const g of ACCEPTANCE_FIELD_GROUPS) out.set(g, []);
    for (const s of ACCEPTANCE_FIELD_REGISTRY) if (match(s)) out.get(s.group)?.push(s);
    return out;
  }, [query]);

  const onDragStart =
    (field: AcceptanceFieldSpec["field"]) =>
    (e: React.DragEvent<HTMLDivElement>): void => {
      e.dataTransfer.setData("text/plain", encodeAcceptanceDrag(field));
      e.dataTransfer.effectAllowed = "copy";
    };

  return (
    <aside className={rr.palette} aria-label="Lift field palette">
      <div className={rr.paletteHead}>
        <h2 className={rr.paletteTitle}>Lift fields</h2>
        <input
          className={rr.paletteSearch}
          type="search"
          placeholder="Search fields…"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          aria-label="Search lift fields"
        />
      </div>

      {ACCEPTANCE_FIELD_GROUPS.map((group) => {
        const items = grouped.get(group) ?? [];
        if (items.length === 0) return null;
        return (
          <section key={group} className={rr.paletteGroup}>
            <h3 className={rr.paletteGroupLabel}>{group}</h3>
            {items.map((s) => (
              <div
                key={s.field}
                className={`${rr.chip} ${rr.chipField}`}
                draggable={!readOnly}
                onDragStart={onDragStart(s.field)}
                aria-label={`Drag ${s.label} into the conditions`}
                data-testid={`acceptance-palette-${s.field}`}
              >
                <span className={`${rr.chipKind} ${rr[`kind_${s.kind}`]}`}>{KIND_TAG[s.kind]}</span>
                <span className={rr.chipMain}>
                  <span className={rr.chipLabel}>{s.label}</span>
                  <span className={rr.chipHint}>{s.hint}</span>
                </span>
              </div>
            ))}
          </section>
        );
      })}

      {readOnly && (
        <p className={rr.paletteReadonly}>
          Read-only — the `manage_acceptance` capability is required to edit.
        </p>
      )}
    </aside>
  );
}
