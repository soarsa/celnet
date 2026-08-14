/**
 * The hedge risk-state field palette — the exact analogue of the risk-routing
 * {@link ../riskrouting/FieldPalette}, listing the {@link HedgeField} vocabulary
 * (docs/AUTO-HEDGING §5.2) instead of the routing trade fields. A trader drags a
 * chip into the rule editor's conditions area to add a `field op value` leg. Drag
 * uses the same HTML5 `dataTransfer` protocol; there is no book/leaf chip (the hedge
 * leaf is a dedicated exit-action editor, not a dragged chip).
 */
import { useMemo, useState } from "react";

import { HelpButton } from "../../components/HelpButton";
import {
  HEDGE_FIELD_GROUPS,
  HEDGE_FIELD_REGISTRY,
  type HedgeFieldGroup,
  type HedgeFieldSpec,
} from "../../lib/hedgeFields";
import rr from "../riskrouting/RiskRoutingWorkspace.module.css";

/** The drag payload written to `text/plain` (parsed by the drop handler). */
export function encodeHedgeDrag(field: HedgeFieldSpec["field"]): string {
  return `hfield:${field}`;
}

/** Decode a hedge field drop payload (null for anything unrecognised). */
export function decodeHedgeDrag(raw: string): HedgeFieldSpec["field"] | null {
  if (!raw.startsWith("hfield:")) return null;
  const field = raw.slice("hfield:".length);
  return HEDGE_FIELD_REGISTRY.some((s) => s.field === field)
    ? (field as HedgeFieldSpec["field"])
    : null;
}

const KIND_TAG: Record<HedgeFieldSpec["kind"], string> = {
  enum: "enum",
  numeric: "num",
  string: "text",
};

interface HedgeFieldPaletteProps {
  readOnly: boolean;
}

export function HedgeFieldPalette({ readOnly }: HedgeFieldPaletteProps): React.ReactElement {
  const [query, setQuery] = useState("");

  const grouped = useMemo(() => {
    const q = query.trim().toLowerCase();
    const match = (s: HedgeFieldSpec): boolean =>
      q.length === 0 || s.label.toLowerCase().includes(q) || s.field.includes(q);
    const out = new Map<HedgeFieldGroup, HedgeFieldSpec[]>();
    for (const g of HEDGE_FIELD_GROUPS) out.set(g, []);
    for (const s of HEDGE_FIELD_REGISTRY) if (match(s)) out.get(s.group)?.push(s);
    return out;
  }, [query]);

  const onDragStart =
    (field: HedgeFieldSpec["field"]) =>
    (e: React.DragEvent<HTMLDivElement>): void => {
      e.dataTransfer.setData("text/plain", encodeHedgeDrag(field));
      e.dataTransfer.effectAllowed = "copy";
    };

  return (
    <aside className={rr.palette} aria-label="Risk-state field palette">
      <div className={rr.paletteHead}>
        <h2 className={rr.paletteTitle}>
          Risk-state fields{" "}
          <HelpButton
            helpId="concept.hedge-field-availability"
            subject="why some fields are unavailable"
          />
        </h2>
        <input
          className={rr.paletteSearch}
          type="search"
          placeholder="Search fields…"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          aria-label="Search risk-state fields"
        />
      </div>

      {HEDGE_FIELD_GROUPS.map((group) => {
        const items = grouped.get(group) ?? [];
        if (items.length === 0) return null;
        return (
          <section key={group} className={rr.paletteGroup}>
            <h3 className={rr.paletteGroupLabel}>{group}</h3>
            {items.map((s) => {
              // A field nothing populates is shown, NOT hidden: hiding it leaves a trader
              // hunting for a chip that a colleague's older policy still references, with
              // no explanation. It is undraggable and states its own reason instead.
              const unavailable = s.provider.state === "unprovided" ? s.provider.reason : null;
              const draggable = !readOnly && unavailable === null;
              return (
                <div
                  key={s.field}
                  className={`${rr.chip} ${rr.chipField}`}
                  draggable={draggable}
                  onDragStart={draggable ? onDragStart(s.field) : undefined}
                  aria-disabled={unavailable !== null}
                  data-unavailable={unavailable !== null ? "true" : undefined}
                  title={unavailable ?? undefined}
                  aria-label={
                    unavailable === null
                      ? `Drag ${s.label} into the conditions`
                      : `${s.label} is unavailable — ${unavailable}`
                  }
                  data-testid={`hedge-palette-${s.field}`}
                >
                  <span className={`${rr.chipKind} ${rr[`kind_${s.kind}`]}`}>
                    {unavailable === null ? KIND_TAG[s.kind] : "n/a"}
                  </span>
                  <span className={rr.chipMain}>
                    <span className={rr.chipLabel}>{s.label}</span>
                    <span className={rr.chipHint}>{unavailable ?? s.hint}</span>
                  </span>
                </div>
              );
            })}
          </section>
        );
      })}

      {readOnly && (
        <p className={rr.paletteReadonly}>Read-only — the `hedge` capability is required to edit.</p>
      )}
    </aside>
  );
}
