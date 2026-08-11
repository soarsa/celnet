/**
 * PricingFeatureCard — one draggable feature card in the pricing-group pipeline
 * canvas (docs/FI-PRICING-GROUPS-DESIGN.md §8.5). Renders the card header (kind
 * badge + keyboard reorder up/down + remove) and, when expanded, the per-kind
 * inline configuration:
 *   MID SHIFT  → shift + unit + a nullable reference-price override
 *   TIERING    → the shared {@link TieringEditor} (Flat markup / Inventory skew / …)
 *   AXE        → side + magnitude + unit
 *   POSITION   → κ + sMax
 *   PANIC/SKEW → skew + a triggered flag
 *
 * Drag-and-drop is NATIVE HTML5 (no new dependency, CSP-safe); a keyboard-operable
 * up/down alternative on every card makes reordering non-mouse-only. The card is a
 * controlled component — the parent owns the {@link FeatureSpec} and applies every
 * edit immutably through `onPatch`.
 */

import type { AxeSide, FeatureSpec, TieringConfig } from "../data/contract";
import { TieringEditor } from "../components/TieringEditor";
import { HelpButton } from "../components/HelpButton";
import { FEATURE_HELP_ID } from "../lib/help";
import {
  AXE_SIDE_LABEL,
  AXE_SIDES,
  FEATURE_KIND_HINT,
  FEATURE_KIND_LABEL,
  type FeatureErrors,
} from "../lib/pricingGroups";
import {
  TIERING_SPREAD_UNIT_LABEL,
  TIERING_SPREAD_UNITS,
  type TieringErrors,
} from "../lib/tiering";
import type { TieringSpreadUnit } from "../data/contract";
import styles from "./PricingGroupsWorkspace.module.css";

import { NumberField } from "../components/NumberField";

/** An all-clear tiering error set for a TIERING feature with no errors. */
const NO_TIERING_ERRORS: TieringErrors = { strategies: {}, guardrails: {} };

/** A sensible default reference-price override when the trader first enables it. */
const DEFAULT_REFERENCE_PRICE = 99.55;

interface PricingFeatureCardProps {
  feature: FeatureSpec;
  index: number;
  count: number;
  expanded: boolean;
  readOnly: boolean;
  errors: FeatureErrors | undefined;
  idPrefix: string;
  dragging: boolean;
  dragOver: boolean;
  onToggleExpand: () => void;
  onPatch: (next: Partial<FeatureSpec>) => void;
  onRemove: () => void;
  onMoveUp: () => void;
  onMoveDown: () => void;
  onDragStart: (e: React.DragEvent) => void;
  onDragEnd: () => void;
  onDragOver: (e: React.DragEvent) => void;
  onDrop: (e: React.DragEvent) => void;
}

export function PricingFeatureCard({
  feature,
  index,
  count,
  expanded,
  readOnly,
  errors,
  idPrefix,
  dragging,
  dragOver,
  onToggleExpand,
  onPatch,
  onRemove,
  onMoveUp,
  onMoveDown,
  onDragStart,
  onDragEnd,
  onDragOver,
  onDrop,
}: PricingFeatureCardProps): React.ReactElement {
  const label = FEATURE_KIND_LABEL[feature.kind];
  const cardClass = [styles.card, dragging ? styles.cardDragging : "", dragOver ? styles.cardOver : ""]
    .filter(Boolean)
    .join(" ");

  const setUnit = (unit: TieringSpreadUnit): void => onPatch({ unit });

  return (
    <div
      className={cardClass}
      draggable={!readOnly}
      onDragStart={onDragStart}
      onDragEnd={onDragEnd}
      onDragOver={onDragOver}
      onDrop={onDrop}
      data-testid={`feature-card-${index}`}
      data-kind={feature.kind}
    >
      <div className={styles.cardHead}>
        <button
          type="button"
          className={styles.cardKindBtn}
          onClick={onToggleExpand}
          aria-expanded={expanded}
          aria-label={`${label} feature, position ${index + 1} of ${count} — ${expanded ? "collapse" : "expand"} configuration`}
        >
          <span className={styles.cardKind}>
            <span className={styles.cardOrder}>{index + 1}</span>
            <span className={styles.cardBadge}>{label}</span>
            <span className={styles.cardCaret} aria-hidden="true">
              {expanded ? "▾" : "▸"}
            </span>
          </span>
        </button>
        <span className={styles.cardControls}>
          <HelpButton helpId={FEATURE_HELP_ID[feature.kind]} subject={label} />
          <button
            type="button"
            className={styles.iconBtn}
            onClick={(e) => {
              e.stopPropagation();
              onMoveUp();
            }}
            disabled={readOnly || index === 0}
            aria-label={`Move ${label} up`}
            title="Move up"
          >
            ↑
          </button>
          <button
            type="button"
            className={styles.iconBtn}
            onClick={(e) => {
              e.stopPropagation();
              onMoveDown();
            }}
            disabled={readOnly || index === count - 1}
            aria-label={`Move ${label} down`}
            title="Move down"
          >
            ↓
          </button>
          <button
            type="button"
            className={`${styles.iconBtn} ${styles.iconBtnDanger}`}
            onClick={(e) => {
              e.stopPropagation();
              onRemove();
            }}
            disabled={readOnly}
            aria-label={`Remove ${label}`}
            title="Remove"
          >
            ×
          </button>
        </span>
      </div>

      {expanded && (
        <div className={styles.cardBody}>
          <p className={styles.cardHint}>{FEATURE_KIND_HINT[feature.kind]}</p>

          {feature.kind === "MID_SHIFT" && (
            <MidShiftFields
              feature={feature}
              readOnly={readOnly}
              errors={errors}
              idPrefix={idPrefix}
              onPatch={onPatch}
              onSetUnit={setUnit}
            />
          )}

          {feature.kind === "TIERING" && (
            <TieringEditor
              value={feature.tiering}
              onChange={(tiering: TieringConfig | null) => onPatch({ tiering })}
              errors={errors?.tiering ?? NO_TIERING_ERRORS}
              idPrefix={`${idPrefix}-tiering`}
            />
          )}

          {feature.kind === "AXE" && (
            <div className={styles.grid}>
              <label className={styles.param} htmlFor={`${idPrefix}-axeside`}>
                <span className={styles.paramLabel}>Side</span>
                <select
                  id={`${idPrefix}-axeside`}
                  className={styles.select}
                  value={feature.axeSide}
                  disabled={readOnly}
                  onChange={(e) => onPatch({ axeSide: e.target.value as AxeSide })}
                >
                  {AXE_SIDES.map((s) => (
                    <option key={s} value={s}>
                      {AXE_SIDE_LABEL[s]}
                    </option>
                  ))}
                </select>
              </label>
              <NumField
                id={`${idPrefix}-mag`}
                label="Magnitude"
                value={feature.magnitude}
                readOnly={readOnly}
                error={errors?.magnitude}
                onChange={(magnitude) => onPatch({ magnitude })}
              />
              <UnitField
                id={`${idPrefix}-axeunit`}
                value={feature.unit}
                readOnly={readOnly}
                onChange={setUnit}
              />
            </div>
          )}

          {feature.kind === "POSITION" && (
            <div className={styles.grid}>
              <NumField
                id={`${idPrefix}-kappa`}
                label="κ (per unit inventory)"
                value={feature.kappa}
                readOnly={readOnly}
                error={errors?.kappa}
                onChange={(kappa) => onPatch({ kappa })}
              />
              <NumField
                id={`${idPrefix}-sMax`}
                label="sMax (skew clamp)"
                value={feature.sMax}
                readOnly={readOnly}
                error={errors?.sMax}
                onChange={(sMax) => onPatch({ sMax })}
              />
            </div>
          )}

          {feature.kind === "PANIC_SKEW" && (
            <div className={styles.grid}>
              <NumField
                id={`${idPrefix}-skew`}
                label="Emergency skew"
                value={feature.skew}
                readOnly={readOnly}
                error={errors?.skew}
                onChange={(skew) => onPatch({ skew })}
              />
              <label className={styles.checkboxRow} htmlFor={`${idPrefix}-trig`}>
                <input
                  id={`${idPrefix}-trig`}
                  type="checkbox"
                  checked={feature.triggered}
                  disabled={readOnly}
                  onChange={(e) => onPatch({ triggered: e.target.checked })}
                />
                <span>Triggered (overlay active)</span>
              </label>
            </div>
          )}
        </div>
      )}
    </div>
  );
}

interface MidShiftFieldsProps {
  feature: FeatureSpec;
  readOnly: boolean;
  errors: FeatureErrors | undefined;
  idPrefix: string;
  onPatch: (next: Partial<FeatureSpec>) => void;
  onSetUnit: (unit: TieringSpreadUnit) => void;
}

function MidShiftFields({
  feature,
  readOnly,
  errors,
  idPrefix,
  onPatch,
  onSetUnit,
}: MidShiftFieldsProps): React.ReactElement {
  const usesReference = feature.reference !== null;
  return (
    <div className={styles.grid}>
      <NumField
        id={`${idPrefix}-shift`}
        label="Shift"
        value={feature.shift}
        readOnly={readOnly || usesReference}
        error={errors?.shift}
        onChange={(shift) => onPatch({ shift })}
      />
      <UnitField
        id={`${idPrefix}-unit`}
        value={feature.unit}
        readOnly={readOnly}
        onChange={onSetUnit}
      />
      <label className={styles.checkboxRow} htmlFor={`${idPrefix}-useref`}>
        <input
          id={`${idPrefix}-useref`}
          type="checkbox"
          checked={usesReference}
          disabled={readOnly}
          onChange={(e) => onPatch({ reference: e.target.checked ? DEFAULT_REFERENCE_PRICE : null })}
        />
        <span>Override mid with a reference price</span>
      </label>
      {usesReference && (
        <NumField
          id={`${idPrefix}-ref`}
          label="Reference price"
          value={feature.reference ?? 0}
          readOnly={readOnly}
          error={errors?.reference}
          onChange={(reference) => onPatch({ reference })}
        />
      )}
    </div>
  );
}

interface NumFieldProps {
  id: string;
  label: string;
  value: number;
  readOnly: boolean;
  error?: string | undefined;
  onChange: (value: number) => void;
}

function NumField({ id, label, value, readOnly, error, onChange }: NumFieldProps): React.ReactElement {
  return (
    <label className={styles.param} htmlFor={id}>
      <span className={styles.paramLabel}>{label}</span>
      <NumberField
        id={id}
        className={`${styles.input} ${styles.numInput} ${error ? styles.inputError : ""}`}
        step="any"
        value={value}
        disabled={readOnly}
        aria-invalid={error ? true : undefined}
        onChange={(e) => onChange(Number(e.target.value))}
      />
      {error && <span className={styles.error}>{error}</span>}
    </label>
  );
}

interface UnitFieldProps {
  id: string;
  value: TieringSpreadUnit;
  readOnly: boolean;
  onChange: (unit: TieringSpreadUnit) => void;
}

function UnitField({ id, value, readOnly, onChange }: UnitFieldProps): React.ReactElement {
  return (
    <label className={styles.param} htmlFor={id}>
      <span className={styles.paramLabel}>Unit</span>
      <select
        id={id}
        className={styles.select}
        value={value}
        disabled={readOnly}
        onChange={(e) => onChange(e.target.value as TieringSpreadUnit)}
      >
        {TIERING_SPREAD_UNITS.map((u) => (
          <option key={u} value={u}>
            {TIERING_SPREAD_UNIT_LABEL[u]}
          </option>
        ))}
      </select>
    </label>
  );
}
