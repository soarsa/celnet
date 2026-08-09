/**
 * CurveDefinitionEditor — create or edit the METADATA of a {@link CurveDefinition}
 * (server commit 38bcff9a): display name, currency, index, day-count, calendar, and
 * the now-WIRE-REAL interpolation scheme (log-linear DF ↔ monotone-convex forward,
 * `celnet.wire.CurveInterpolation`). On create it mints an IMMUTABLE slug id from
 * the display name (editable until first save) and seeds a real starter pillar
 * ladder the trader refines in the Pillars tab; on edit the slug is fixed and the
 * pillar ladder is preserved. Persists through the one contract (create / update),
 * surfacing the server's `already_exists` / `invalid_argument` guidance.
 *
 * Capability-gated: without Refdata·Fixed-Income the whole form renders read-only
 * (inputs disabled, no Save) — a viewer inspects the definition but cannot mutate it.
 */

import { useMemo, useState } from "react";

import { Button } from "../components/Button";
import { HelpButton } from "../components/HelpButton";
import type { CurveDefinition, CurveInterpolation } from "../data/contract";
import {
  CURVE_INTERPOLATIONS,
  curveInterpolationLabel,
} from "../data/contract";
import { slugifyCurveId, starterPillarSet } from "../lib/curveEditing";
import styles from "./CurveWorkspace.module.css";

export interface CurveDefinitionEditorProps {
  /** The definition being edited, or `null` to create a new curve. */
  existing: CurveDefinition | null;
  /** True when the identity holds Refdata·Fixed-Income (may persist). */
  canEdit: boolean;
  /** Persist a new curve; resolves to the stored record or rejects with a coded error. */
  onCreate: (definition: CurveDefinition) => Promise<CurveDefinition>;
  /** Replace the identified definition; resolves or rejects with a coded error. */
  onUpdate: (
    curveId: string,
    definition: CurveDefinition,
  ) => Promise<CurveDefinition>;
  /** Called with the stored curve id after a successful save. */
  onSaved: (curveId: string) => void;
  /** Abandon the edit (back to the dashboard). */
  onCancel: () => void;
}

/** Narrow a thrown value to a display string. */
function messageOf(error: unknown): string {
  return error instanceof Error ? error.message : "curve save failed";
}

export function CurveDefinitionEditor({
  existing,
  canEdit,
  onCreate,
  onUpdate,
  onSaved,
  onCancel,
}: CurveDefinitionEditorProps): React.ReactElement {
  const isCreate = existing === null;

  const [displayName, setDisplayName] = useState(existing?.displayName ?? "");
  const [currency, setCurrency] = useState(
    existing?.pillars.currency ?? "USD",
  );
  const [indexLabel, setIndexLabel] = useState(
    existing?.indexLabel ?? "USD-SOFR",
  );
  const [dayCount, setDayCount] = useState(existing?.dayCount ?? "ACT/360");
  const [calendar, setCalendar] = useState(existing?.calendar ?? "USD");
  const [interpolation, setInterpolation] = useState<CurveInterpolation>(
    existing?.interpolation ?? "log-linear-df",
  );
  const [primary, setPrimary] = useState(existing?.primary ?? false);

  // The slug: auto-derived from the display name until the trader edits it by hand
  // (create only — the slug is immutable once stored). `slugTouched` freezes the
  // auto-derivation the moment the user types in the id field.
  const [slugTouched, setSlugTouched] = useState(false);
  const [slugInput, setSlugInput] = useState(existing?.curveId ?? "");
  const slug = isCreate
    ? slugTouched
      ? slugInput
      : slugifyCurveId(displayName)
    : existing.curveId;

  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const trimmedName = displayName.trim();
  const validationError = useMemo<string | null>(() => {
    if (trimmedName.length === 0) return "A display name is required.";
    if (isCreate && slug.length === 0)
      return "A curve id (slug) is required — it becomes the immutable key.";
    if (currency.trim().length !== 3)
      return "Currency must be a 3-letter ISO code.";
    return null;
  }, [trimmedName, isCreate, slug, currency]);

  const buildDefinition = (): CurveDefinition => {
    const ccy = currency.trim().toUpperCase();
    // On create, seed a real bootstrappable starter ladder in the chosen currency;
    // on edit, preserve the curve's own pillar ladder (re-homing only its currency).
    const pillars = isCreate
      ? starterPillarSet(ccy)
      : { ...existing.pillars, currency: ccy };
    return {
      curveId: slug,
      displayName: trimmedName,
      indexLabel: indexLabel.trim(),
      dayCount: dayCount.trim(),
      calendar: calendar.trim(),
      interpolation,
      pillars,
      primary,
    };
  };

  const save = async (): Promise<void> => {
    if (validationError) {
      setError(validationError);
      return;
    }
    setSaving(true);
    setError(null);
    try {
      const def = buildDefinition();
      const stored = isCreate
        ? await onCreate(def)
        : await onUpdate(existing.curveId, def);
      onSaved(stored.curveId);
    } catch (e: unknown) {
      setError(messageOf(e));
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className={styles.definitionPane}>
      <div className={styles.dashHead}>
        <div className={styles.dashHeadMain}>
          <h2 className={styles.dashTitle}>
            {isCreate ? "New curve" : `Edit ${existing.displayName}`}
          </h2>
          <HelpButton
            helpId="concept.curve-definitions"
            subject="the curve reference-data fields"
          />
        </div>
        <Button variant="ghost" onClick={onCancel} title="back to the dashboard">
          ← Dashboard
        </Button>
      </div>

      {!canEdit && (
        <p className={styles.notice} role="status">
          You are viewing this definition read-only — creating and editing curves
          needs the Reference-data (Fixed Income) capability.
        </p>
      )}

      <div className={styles.defForm}>
        <label className={styles.defField}>
          <span className={styles.fieldLabel}>Display name</span>
          <input
            type="text"
            className={styles.defInput}
            value={displayName}
            disabled={!canEdit}
            aria-label="curve display name"
            placeholder="USD SOFR"
            onChange={(e) => setDisplayName(e.target.value)}
          />
        </label>

        <label className={styles.defField}>
          <span className={styles.fieldLabel}>Curve id (slug)</span>
          <input
            type="text"
            className={styles.defInput}
            value={slug}
            disabled={!canEdit || !isCreate}
            aria-label="curve id slug"
            placeholder="usd-sofr"
            onChange={(e) => {
              setSlugTouched(true);
              setSlugInput(slugifyCurveId(e.target.value));
            }}
          />
          <span className={styles.defHint}>
            {isCreate
              ? "Immutable once created — generated from the name; edit to override."
              : "Immutable — the slug never changes."}
          </span>
        </label>

        <label className={styles.defField}>
          <span className={styles.fieldLabel}>Currency</span>
          <input
            type="text"
            className={`${styles.defInput} ${styles.ccyInput}`}
            value={currency}
            disabled={!canEdit}
            maxLength={3}
            aria-label="curve currency"
            onChange={(e) => setCurrency(e.target.value.toUpperCase().slice(0, 3))}
          />
        </label>

        <label className={styles.defField}>
          <span className={styles.fieldLabel}>Index</span>
          <input
            type="text"
            className={styles.defInput}
            value={indexLabel}
            disabled={!canEdit}
            aria-label="curve index label"
            placeholder="USD-SOFR"
            onChange={(e) => setIndexLabel(e.target.value)}
          />
        </label>

        <label className={styles.defField}>
          <span className={styles.fieldLabel}>Day count</span>
          <input
            type="text"
            className={styles.defInput}
            value={dayCount}
            disabled={!canEdit}
            aria-label="curve day-count convention"
            placeholder="ACT/360"
            onChange={(e) => setDayCount(e.target.value)}
          />
        </label>

        <label className={styles.defField}>
          <span className={styles.fieldLabel}>Calendar</span>
          <input
            type="text"
            className={styles.defInput}
            value={calendar}
            disabled={!canEdit}
            aria-label="curve holiday calendar"
            placeholder="USD"
            onChange={(e) => setCalendar(e.target.value)}
          />
        </label>
      </div>

      <fieldset className={styles.modelField}>
        <legend className={styles.fieldLabel}>
          Interpolation
          <span className={styles.legendHelp}>
            <HelpButton
              helpId="concept.curve-interpolation"
              subject="the curve interpolation choices"
            />
          </span>
        </legend>
        <div className={styles.modelChoices}>
          {CURVE_INTERPOLATIONS.map((scheme) => (
            <label key={scheme} className={styles.modelChoice}>
              <input
                type="radio"
                name="curve-interpolation"
                value={scheme}
                checked={interpolation === scheme}
                disabled={!canEdit}
                aria-label={curveInterpolationLabel(scheme)}
                onChange={() => setInterpolation(scheme)}
              />
              <span>{curveInterpolationLabel(scheme)}</span>
            </label>
          ))}
        </div>
        <p className={styles.modelNote}>
          Log-linear on the discount factor (the default) gives piecewise-constant
          forwards; monotone-convex shapes the instantaneous forward. The engine
          bootstraps the persisted curve with the scheme you pick.
        </p>
      </fieldset>

      <label className={styles.defCheck}>
        <input
          type="checkbox"
          checked={primary}
          disabled={!canEdit}
          aria-label="make this the primary curve for its currency"
          onChange={(e) => setPrimary(e.target.checked)}
        />
        <span>
          Primary (default) curve for {currency || "its currency"}
          <span className={styles.defHint}>
            {" "}
            — exactly one per currency; setting this demotes the current primary.
          </span>
        </span>
      </label>

      {error && (
        <p className={styles.error} role="alert">
          {error}
        </p>
      )}

      {canEdit && (
        <div className={styles.buildRow}>
          <Button
            variant="primary"
            onClick={() => void save()}
            disabled={saving || validationError !== null}
            title={
              validationError ?? (isCreate ? "create this curve" : "save changes")
            }
          >
            {saving ? "Saving…" : isCreate ? "Create curve" : "Save changes"}
          </Button>
        </div>
      )}
    </div>
  );
}
