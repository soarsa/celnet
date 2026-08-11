/**
 * The typed value editor for one routing condition — shared by the rule editor.
 * Renders the right control for the operator + field kind: `between` ⇒ two numeric
 * bounds; `in` ⇒ a membership editor (enum chips or a free tag input); numeric
 * single-value ops ⇒ a number input; enum equality with a live source ⇒ a dropdown;
 * everything else ⇒ a text input. Every change emits a fully-formed
 * {@link RouteValue} whose variant matches the operator.
 */
import type { RouteOp, RouteValue } from "../../data/contract";
import type { FieldKind } from "../../lib/routeFields";
import styles from "./RiskRoutingWorkspace.module.css";

import { NumberField } from "../../components/NumberField";

/** One selectable enum value (value id + display label). */
export interface ValueOption {
  value: string;
  label: string;
}

interface ValueEditorProps {
  op: RouteOp;
  value: RouteValue | null;
  kind: FieldKind;
  enumOptions: ValueOption[];
  readOnly: boolean;
  onChange: (v: RouteValue) => void;
}

export function ValueEditor({
  op,
  value,
  kind,
  enumOptions,
  readOnly,
  onChange,
}: ValueEditorProps): React.ReactElement {
  // `between` — two numeric bounds.
  if (op === "between") {
    const lo = value?.kind === "range" ? value.lo : 0;
    const hi = value?.kind === "range" ? value.hi : 0;
    return (
      <div className={styles.editorField}>
        <span className={styles.fieldLabel}>Range (inclusive)</span>
        <div className={styles.rangeRow}>
          <NumberField
            className={styles.input}
            value={lo}
            disabled={readOnly}
            aria-label="Lower bound"
            onChange={(e) => onChange({ kind: "range", lo: Number(e.target.value), hi })}
          />
          <span className={styles.rangeSep}>to</span>
          <NumberField
            className={styles.input}
            value={hi}
            disabled={readOnly}
            aria-label="Upper bound"
            onChange={(e) => onChange({ kind: "range", lo, hi: Number(e.target.value) })}
          />
        </div>
      </div>
    );
  }

  // `in` — membership. Enum source ⇒ toggle chips; else a tag input.
  if (op === "in") {
    const values = value?.kind === "list" ? value.values : [];
    return (
      <MultiValueEditor
        values={values}
        enumOptions={enumOptions}
        readOnly={readOnly}
        numeric={kind === "numeric"}
        onChange={(vs) => onChange({ kind: "list", values: vs })}
      />
    );
  }

  // Numeric single-value ops.
  if (kind === "numeric") {
    const num = value?.kind === "num" ? value.num : 0;
    return (
      <label className={styles.editorField}>
        <span className={styles.fieldLabel}>Value</span>
        <NumberField
          className={styles.input}
          value={num}
          disabled={readOnly}
          data-testid="value-num"
          onChange={(e) => onChange({ kind: "num", num: Number(e.target.value) })}
        />
      </label>
    );
  }

  // Enum single-value (eq/ne) with a live source ⇒ dropdown; else free text.
  const text = value?.kind === "text" ? value.text : "";
  if (kind === "enum" && enumOptions.length > 0 && op !== "contains") {
    const known = enumOptions.some((o) => o.value === text);
    return (
      <label className={styles.editorField}>
        <span className={styles.fieldLabel}>Value</span>
        <select
          className={styles.select}
          value={text}
          disabled={readOnly}
          data-testid="value-enum"
          onChange={(e) => onChange({ kind: "text", text: e.target.value })}
        >
          <option value="">(select)</option>
          {!known && text.length > 0 && <option value={text}>{text}</option>}
          {enumOptions.map((o) => (
            <option key={o.value} value={o.value}>
              {o.label}
            </option>
          ))}
        </select>
      </label>
    );
  }

  // String (or enum with no source) ⇒ text input.
  return (
    <label className={styles.editorField}>
      <span className={styles.fieldLabel}>Value</span>
      <input
        className={styles.input}
        type="text"
        value={text}
        disabled={readOnly}
        data-testid="value-text"
        onChange={(e) => onChange({ kind: "text", text: e.target.value })}
      />
    </label>
  );
}

function MultiValueEditor({
  values,
  enumOptions,
  readOnly,
  numeric,
  onChange,
}: {
  values: string[];
  enumOptions: ValueOption[];
  readOnly: boolean;
  numeric: boolean;
  onChange: (vs: string[]) => void;
}): React.ReactElement {
  const add = (v: string): void => {
    const t = v.trim();
    if (t.length === 0 || values.includes(t)) return;
    onChange([...values, t]);
  };
  const remove = (v: string): void => onChange(values.filter((x) => x !== v));

  return (
    <div className={styles.editorField}>
      <span className={styles.fieldLabel}>In list</span>
      <div className={styles.tagRow}>
        {values.length === 0 && <span className={styles.tagEmpty}>No values yet</span>}
        {values.map((v) => (
          <span key={v} className={styles.tag}>
            {v}
            {!readOnly && (
              <button
                type="button"
                className={styles.tagX}
                aria-label={`Remove ${v}`}
                onClick={() => remove(v)}
              >
                ×
              </button>
            )}
          </span>
        ))}
      </div>
      {!readOnly && enumOptions.length > 0 && (
        <select
          className={styles.select}
          value=""
          data-testid="multi-enum-add"
          onChange={(e) => {
            if (e.target.value) add(e.target.value);
          }}
        >
          <option value="">+ add value…</option>
          {enumOptions
            .filter((o) => !values.includes(o.value))
            .map((o) => (
              <option key={o.value} value={o.value}>
                {o.label}
              </option>
            ))}
        </select>
      )}
      {!readOnly && enumOptions.length === 0 && (
        <input
          className={styles.input}
          type={numeric ? "number" : "text"}
          placeholder={numeric ? "Add a number, press Enter" : "Add a value, press Enter"}
          data-testid="multi-free-add"
          onKeyDown={(e) => {
            if (e.key === "Enter") {
              e.preventDefault();
              add((e.target as HTMLInputElement).value);
              (e.target as HTMLInputElement).value = "";
            }
          }}
        />
      )}
    </div>
  );
}
