/**
 * RuleEditor — the per-rule builder. A trader drags field chips from the palette
 * into the condition area to add `field op value` legs (ANDed within the rule),
 * rebinds/edits each leg with typed controls, and picks the terminal destination
 * ("Drop into this risk book": desk → that desk's risk book). A live preview shows
 * the rule in plain English. Save validates (a non-default rule needs a destination)
 * and hands the finished {@link RiskRule} back to the table; Cancel discards.
 *
 * This is the primary editing surface — there is NO decision-tree wiring here. The
 * rule's conditions compile to a deterministic first-match-wins graph spine by
 * {@link compileRulesToGraph}; the trader never wires yes/no ports.
 */
import { useMemo, useState } from "react";

import type {
  DeskDesc,
  FixConnection,
  RiskBook,
  RouteField,
  RouteOp,
  RouteValue,
} from "../../data/contract";
import {
  CCY_VALUES,
  FIELD_REGISTRY,
  PRODUCT_VALUES,
  SIDE_VALUES,
  fieldSpec,
  opLabel,
} from "../../lib/routeFields";
import { describeRule, type RiskRule, type RuleCondition } from "../../lib/riskRules";
import { FieldPalette, decodeDrag } from "./FieldPalette";
import { defaultOpForField, defaultValueForOp } from "./graphOps";
import { ValueEditor, type ValueOption } from "./ValueEditor";
import styles from "./RiskRoutingWorkspace.module.css";

/** Sentinel desk value in the book picker for books with no owning desk. */
const UNASSIGNED_DESK = "__unassigned__";

interface RuleEditorProps {
  /** The rule being created/edited (seeds the form; its id + enabled are preserved). */
  draft: RiskRule;
  /** Whether this is a brand-new rule (title/CTA copy) or an edit. */
  isNew: boolean;
  books: readonly RiskBook[];
  desks: readonly DeskDesc[];
  connections: readonly FixConnection[];
  /** Desk-scoped book label resolver (`DESK / BOOK`), for the live preview. */
  bookLabel: (id: string) => string;
  readOnly: boolean;
  onSave: (rule: RiskRule) => void;
  onCancel: () => void;
}

/** The books a desk-filter value selects. */
function booksForDeskId(books: readonly RiskBook[], deskFilter: string): RiskBook[] {
  if (deskFilter === "") return [];
  return books.filter((b) =>
    deskFilter === UNASSIGNED_DESK
      ? b.deskId === null || b.deskId.length === 0
      : b.deskId === deskFilter,
  );
}

export function RuleEditor({
  draft,
  isNew,
  books,
  desks,
  connections,
  bookLabel,
  readOnly,
  onSave,
  onCancel,
}: RuleEditorProps): React.ReactElement {
  const [conditions, setConditions] = useState<RuleCondition[]>(draft.conditions);
  const [bookId, setBookId] = useState<string | null>(draft.bookId);
  const [error, setError] = useState<string | null>(null);

  const deskOfCurrent = useMemo(() => {
    const b = bookId !== null ? books.find((x) => x.id === bookId) : undefined;
    if (!b) return "";
    return b.deskId !== null && b.deskId.length > 0 ? b.deskId : UNASSIGNED_DESK;
  }, [books, bookId]);
  const [deskFilter, setDeskFilter] = useState<string>(deskOfCurrent);

  const isDefault = conditions.length === 0;

  // --- condition mutation ---------------------------------------------------
  const addCondition = (field: RouteField): void => {
    const op = defaultOpForField(field);
    setConditions((cs) => [...cs, { field, op, value: defaultValueForOp(field, op) }]);
    setError(null);
  };
  const removeCondition = (idx: number): void =>
    setConditions((cs) => cs.filter((_, i) => i !== idx));
  const rebindField = (idx: number, field: RouteField): void =>
    setConditions((cs) =>
      cs.map((c, i) => {
        if (i !== idx) return c;
        const op = defaultOpForField(field);
        return { field, op, value: defaultValueForOp(field, op) };
      }),
    );
  const setOp = (idx: number, op: RouteOp): void =>
    setConditions((cs) =>
      cs.map((c, i) => (i === idx ? { ...c, op, value: defaultValueForOp(c.field, op) } : c)),
    );
  const setVal = (idx: number, value: RouteValue): void =>
    setConditions((cs) => cs.map((c, i) => (i === idx ? { ...c, value } : c)));

  // --- destination ----------------------------------------------------------
  const { deskRows, hasUnassigned } = useMemo(() => {
    const owning = new Set<string>();
    let unassigned = false;
    for (const b of books) {
      if (b.deskId !== null && b.deskId.length > 0) owning.add(b.deskId);
      else unassigned = true;
    }
    const rows = desks.filter((d) => owning.has(d.id));
    for (const id of owning) if (!rows.some((d) => d.id === id)) rows.push({ id, name: id });
    return { deskRows: rows, hasUnassigned: unassigned };
  }, [books, desks]);

  const booksForDesk = useMemo(() => booksForDeskId(books, deskFilter), [books, deskFilter]);

  const onDeskChange = (next: string): void => {
    setDeskFilter(next);
    const stillValid = booksForDeskId(books, next).some((b) => b.id === bookId);
    if (!stillValid) setBookId(null);
  };

  // --- preview + save -------------------------------------------------------
  const workingRule: RiskRule = { ...draft, conditions, bookId };
  const preview = describeRule(workingRule, bookLabel);

  const onDropField = (raw: string): void => {
    const payload = decodeDrag(raw);
    if (payload && payload.kind === "field") addCondition(payload.field);
  };

  const handleSave = (): void => {
    if (!isDefault && (bookId === null || bookId.length === 0)) {
      setError("Pick a destination risk portfolio (or remove all conditions for a catch-all rule).");
      return;
    }
    onSave({ ...draft, conditions, bookId });
  };

  return (
    <div className={styles.ruleEditor} role="dialog" aria-label="Risk rule editor" data-testid="rule-editor">
      <div className={styles.ruleEditorPanel}>
        <header className={styles.ruleEditorHead}>
          <div>
            <h2 className={styles.ruleEditorTitle}>{isNew ? "Create risk rule" : "Edit risk rule"}</h2>
            <p className={styles.ruleEditorHint}>
              Drag field chips into the conditions area to build{" "}
              <strong>IF &lt;conditions&gt; THEN route into a risk portfolio</strong>. Remove every
              condition to make this the catch-all (default) rule.
            </p>
          </div>
          <button type="button" className={styles.ghostBtn} onClick={onCancel} aria-label="Close editor">
            ✕
          </button>
        </header>

        <div className={styles.ruleEditorBody}>
          {!readOnly && <FieldPalette readOnly={readOnly} showBookChip={false} />}

          <div className={styles.ruleEditorMain}>
            <section
              className={styles.condArea}
              data-testid="rule-condition-area"
              aria-label="Rule conditions"
              onDragOver={(e) => {
                if (!readOnly) e.preventDefault();
              }}
              onDrop={(e) => {
                e.preventDefault();
                if (readOnly) return;
                onDropField(e.dataTransfer.getData("text/plain"));
              }}
            >
              <h3 className={styles.condAreaTitle}>Conditions (all must hold)</h3>
              {conditions.length === 0 ? (
                <p className={styles.condEmpty}>
                  {readOnly
                    ? "No conditions — this is the catch-all (default) rule."
                    : "Drag a field chip here to add a condition, or leave empty for the catch-all rule."}
                </p>
              ) : (
                <ul className={styles.condList}>
                  {conditions.map((c, idx) => (
                    <ConditionRow
                      key={idx}
                      index={idx}
                      cond={c}
                      desks={desks}
                      connections={connections}
                      readOnly={readOnly}
                      onRebind={rebindField}
                      onSetOp={setOp}
                      onSetVal={setVal}
                      onRemove={removeCondition}
                    />
                  ))}
                </ul>
              )}
            </section>

            <section className={styles.destArea} aria-label="Destination">
              <h3 className={styles.condAreaTitle}>Route into this risk portfolio</h3>
              {isDefault && (
                <p className={styles.condEmpty}>
                  Catch-all rules may route anywhere every unmatched fill should land.
                </p>
              )}
              <div className={styles.destRow}>
                <label className={styles.editorField}>
                  <span className={styles.fieldLabel}>Owning desk</span>
                  <select
                    className={styles.select}
                    value={deskFilter}
                    disabled={readOnly}
                    onChange={(e) => onDeskChange(e.target.value)}
                    data-testid="book-desk-select"
                  >
                    <option value="">(select a desk)</option>
                    {deskRows.map((d) => (
                      <option key={d.id} value={d.id}>
                        {d.name}
                      </option>
                    ))}
                    {hasUnassigned && <option value={UNASSIGNED_DESK}>Unassigned</option>}
                  </select>
                </label>
                <label className={styles.editorField}>
                  <span className={styles.fieldLabel}>Risk portfolio</span>
                  <select
                    className={styles.select}
                    value={bookId ?? ""}
                    disabled={readOnly || deskFilter === ""}
                    onChange={(e) => setBookId(e.target.value.length > 0 ? e.target.value : null)}
                    data-testid="book-target-select"
                  >
                    <option value="">
                      {deskFilter === "" ? "(pick a desk first)" : "(select a portfolio)"}
                    </option>
                    {booksForDesk.map((book) => (
                      <option key={book.id} value={book.id} disabled={!book.enabled}>
                        {`${book.name}${book.enabled ? "" : " (disabled)"}`}
                      </option>
                    ))}
                  </select>
                </label>
              </div>
            </section>

            <p className={styles.rulePreview} data-testid="rule-preview">
              <span className={styles.rulePreviewLabel}>Preview</span>
              {preview}
            </p>
          </div>
        </div>

        {error !== null && (
          <p className={styles.error} role="alert">
            {error}
          </p>
        )}

        <footer className={styles.ruleEditorFoot}>
          <button type="button" className={styles.ghostBtn} onClick={onCancel}>
            Cancel
          </button>
          {!readOnly && (
            <button type="button" className={styles.saveBtn} onClick={handleSave} data-testid="rule-save">
              Save rule
            </button>
          )}
        </footer>
      </div>
    </div>
  );
}

/** Build the enum value options for a field (static lists or live rosters). */
function enumOptionsFor(
  field: RouteField,
  desks: readonly DeskDesc[],
  connections: readonly FixConnection[],
): ValueOption[] {
  const spec = fieldSpec(field);
  switch (spec.enumSource) {
    case "side":
      return SIDE_VALUES.map((v) => ({ value: v, label: v }));
    case "product":
      return PRODUCT_VALUES.map((v) => ({ value: v, label: v }));
    case "ccy":
      return CCY_VALUES.map((v) => ({ value: v, label: v }));
    case "desk":
      return desks.map((d) => ({ value: d.id, label: d.name }));
    case "counterparty":
      return connections.map((c) => ({ value: c.id, label: c.name || c.id }));
    default:
      return [];
  }
}

function ConditionRow({
  index,
  cond,
  desks,
  connections,
  readOnly,
  onRebind,
  onSetOp,
  onSetVal,
  onRemove,
}: {
  index: number;
  cond: RuleCondition;
  desks: readonly DeskDesc[];
  connections: readonly FixConnection[];
  readOnly: boolean;
  onRebind: (idx: number, field: RouteField) => void;
  onSetOp: (idx: number, op: RouteOp) => void;
  onSetVal: (idx: number, value: RouteValue) => void;
  onRemove: (idx: number) => void;
}): React.ReactElement {
  const spec = fieldSpec(cond.field);
  const enumOptions = useMemo(
    () => enumOptionsFor(cond.field, desks, connections),
    [cond.field, desks, connections],
  );

  return (
    <li className={styles.condRow} data-testid={`cond-row-${index}`}>
      {index > 0 && <span className={styles.condAnd}>AND</span>}
      <div className={styles.condControls}>
        <label className={styles.editorField}>
          <span className={styles.fieldLabel}>Field</span>
          <select
            className={styles.select}
            value={cond.field}
            disabled={readOnly}
            data-testid={`cond-field-${index}`}
            onChange={(e) => onRebind(index, e.target.value as RouteField)}
          >
            {FIELD_REGISTRY.map((s) => (
              <option key={s.field} value={s.field}>
                {s.label}
              </option>
            ))}
          </select>
        </label>

        <label className={styles.editorField}>
          <span className={styles.fieldLabel}>Operator</span>
          <select
            className={styles.select}
            value={cond.op}
            disabled={readOnly}
            data-testid={`cond-op-${index}`}
            onChange={(e) => onSetOp(index, e.target.value as RouteOp)}
          >
            {spec.validOps.map((op) => (
              <option key={op} value={op}>
                {opLabel(op)}
              </option>
            ))}
          </select>
        </label>

        <ValueEditor
          op={cond.op}
          value={cond.value}
          kind={spec.kind}
          enumOptions={enumOptions}
          readOnly={readOnly}
          onChange={(v) => onSetVal(index, v)}
        />
      </div>
      {!readOnly && (
        <button
          type="button"
          className={styles.dangerBtn}
          data-testid={`cond-remove-${index}`}
          aria-label={`Remove condition ${index + 1}`}
          onClick={() => onRemove(index)}
        >
          Remove
        </button>
      )}
    </li>
  );
}
