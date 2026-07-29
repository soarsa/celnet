/**
 * The typed editor for the selected node. A Condition node exposes: a field
 * summary, an operator `<select>` offering ONLY the ops legal for the field's kind,
 * a TYPED value editor (enum ⇒ dropdown / multi-toggle of valid values; numeric ⇒
 * number input(s); string ⇒ text), and two branch selects wiring `yes`/`no` to
 * another node. A Book leaf picks its destination from the risk-book tree. Enum
 * value dropdowns are sourced live: side (Buy/Sell), product, ccy, desk (desks),
 * counterparty (FIX connections). Read-only for non-admins.
 */
import { useMemo, useState } from "react";

import type {
  DeskDesc,
  FixConnection,
  RiskBook,
  RiskRoutingGraph,
  RouteOp,
  RouteValue,
  RoutingNode,
} from "../../data/contract";
import {
  CCY_VALUES,
  PRODUCT_VALUES,
  SIDE_VALUES,
  fieldSpec,
  opLabel,
} from "../../lib/routeFields";
import type { ValidationIssue } from "../../lib/routeTrace";
import { makeBookLabel, nodeSummary } from "./nodeLabel";
import styles from "./RiskRoutingWorkspace.module.css";

/** Sentinel desk value in the book picker for books with no owning desk. */
const UNASSIGNED_DESK = "__unassigned__";

interface NodeEditorProps {
  node: RoutingNode;
  graph: RiskRoutingGraph;
  books: readonly RiskBook[];
  desks: readonly DeskDesc[];
  connections: readonly FixConnection[];
  issues: readonly ValidationIssue[];
  readOnly: boolean;
  isEntry: boolean;
  onSetOperator: (nodeId: number, op: RouteOp) => void;
  onSetValue: (nodeId: number, value: RouteValue) => void;
  onSetBranch: (nodeId: number, branch: "onTrue" | "onFalse", target: number) => void;
  onSetBookTarget: (nodeId: number, bookId: string) => void;
  onSetEntry: (nodeId: number) => void;
  onDelete: (nodeId: number) => void;
}

interface Opt {
  value: string;
  label: string;
}

export function NodeEditor(props: NodeEditorProps): React.ReactElement {
  const { node, graph, books, desks, connections, issues, readOnly, isEntry } = props;

  const bookName = useMemo(() => makeBookLabel(books, desks), [books, desks]);

  return (
    <section className={styles.editor} aria-label={`Editor for node ${node.id}`}>
      <header className={styles.editorHead}>
        <div>
          <h2 className={styles.editorTitle}>
            {node.kind === "book" ? "Book destination" : "Condition"}
            <span className={styles.editorId}>#{node.id}</span>
          </h2>
          <p className={styles.editorSummary}>{nodeSummary(node, bookName)}</p>
        </div>
        {!readOnly && (
          <div className={styles.editorHeadActions}>
            {!isEntry && (
              <button
                type="button"
                className={styles.ghostBtn}
                onClick={() => props.onSetEntry(node.id)}
                title="Make this the entry node"
              >
                Set entry
              </button>
            )}
            <button
              type="button"
              className={styles.dangerBtn}
              onClick={() => props.onDelete(node.id)}
            >
              Delete
            </button>
          </div>
        )}
      </header>

      {issues.length > 0 && (
        <ul className={styles.editorIssues}>
          {issues.map((i, idx) => (
            <li key={idx} className={styles.editorIssue} role="alert">
              {i.message}
            </li>
          ))}
        </ul>
      )}

      {node.kind === "book" ? (
        <BookEditor
          key={node.id}
          node={node}
          books={books}
          desks={desks}
          readOnly={readOnly}
          onSet={props.onSetBookTarget}
        />
      ) : (
        <ConditionEditor
          node={node}
          graph={graph}
          desks={desks}
          connections={connections}
          books={books}
          readOnly={readOnly}
          onSetOperator={props.onSetOperator}
          onSetValue={props.onSetValue}
          onSetBranch={props.onSetBranch}
        />
      )}
    </section>
  );
}

// --- Book leaf editor ------------------------------------------------------

function BookEditor({
  node,
  books,
  desks,
  readOnly,
  onSet,
}: {
  node: Extract<RoutingNode, { kind: "book" }>;
  books: readonly RiskBook[];
  desks: readonly DeskDesc[];
  readOnly: boolean;
  onSet: (nodeId: number, bookId: string) => void;
}): React.ReactElement {
  // The destination is DESK-SCOPED: first pick the owning desk, then a book from
  // that desk's portfolio. Books with no owning desk sit under "Unassigned". The
  // stored value stays the bare `bookId` (the wire is unchanged) — this is purely
  // how the destination is PICKED, making it obvious you route risk into a desk's
  // portfolio.
  const deskOfCurrent = useMemo(() => {
    const b = books.find((x) => x.id === node.bookId);
    if (!b) return "";
    return b.deskId !== null && b.deskId.length > 0 ? b.deskId : UNASSIGNED_DESK;
  }, [books, node.bookId]);

  // The desk currently filtering the book list; seeded from the stored book's owner
  // (empty when nothing is chosen yet). Remounted per node via the parent's `key`.
  const [deskFilter, setDeskFilter] = useState<string>(deskOfCurrent);

  // Which desks actually own books + whether any unowned book exists.
  const { deskRows, hasUnassigned } = useMemo(() => {
    const owning = new Set<string>();
    let unassigned = false;
    for (const b of books) {
      if (b.deskId !== null && b.deskId.length > 0) owning.add(b.deskId);
      else unassigned = true;
    }
    const rows = desks.filter((d) => owning.has(d.id));
    // A desk that owns books but is missing from the roster still needs an entry.
    for (const id of owning) {
      if (!rows.some((d) => d.id === id)) rows.push({ id, name: id });
    }
    return { deskRows: rows, hasUnassigned: unassigned };
  }, [books, desks]);

  const booksForDesk = useMemo(() => booksForDeskId(books, deskFilter), [books, deskFilter]);

  const onDeskChange = (next: string): void => {
    setDeskFilter(next);
    // Drop a now-out-of-desk book so the stored value never contradicts the picker.
    const stillValid = booksForDeskId(books, next).some((b) => b.id === node.bookId);
    if (!stillValid) onSet(node.id, "");
  };

  return (
    <>
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
        <span className={styles.fieldLabel}>Route risk to book</span>
        <select
          className={styles.select}
          value={node.bookId}
          disabled={readOnly || deskFilter === ""}
          onChange={(e) => onSet(node.id, e.target.value)}
          data-testid="book-target-select"
        >
          <option value="">
            {deskFilter === "" ? "(pick a desk first)" : "(select a book)"}
          </option>
          {booksForDesk.map((book) => (
            <option key={book.id} value={book.id} disabled={!book.enabled}>
              {`${book.name}${book.enabled ? "" : " (disabled)"}`}
            </option>
          ))}
        </select>
        <span className={styles.fieldHint}>
          A fill landing here books its risk into this desk's portfolio. Only enabled books are
          valid destinations.
        </span>
      </label>
    </>
  );
}

/** The books a desk-filter value selects (shared by the picker + its reset guard). */
function booksForDeskId(books: readonly RiskBook[], deskFilter: string): RiskBook[] {
  if (deskFilter === "") return [];
  return books.filter((b) =>
    deskFilter === UNASSIGNED_DESK
      ? b.deskId === null || b.deskId.length === 0
      : b.deskId === deskFilter,
  );
}

// --- Condition editor ------------------------------------------------------

function ConditionEditor({
  node,
  graph,
  desks,
  connections,
  books,
  readOnly,
  onSetOperator,
  onSetValue,
  onSetBranch,
}: {
  node: Extract<RoutingNode, { kind: "condition" }>;
  graph: RiskRoutingGraph;
  desks: readonly DeskDesc[];
  connections: readonly FixConnection[];
  books: readonly RiskBook[];
  readOnly: boolean;
  onSetOperator: (nodeId: number, op: RouteOp) => void;
  onSetValue: (nodeId: number, value: RouteValue) => void;
  onSetBranch: (nodeId: number, branch: "onTrue" | "onFalse", target: number) => void;
}): React.ReactElement {
  const spec = fieldSpec(node.condition.field);
  const bookName = useMemo(() => makeBookLabel(books, desks), [books, desks]);

  const enumOptions = useMemo((): Opt[] => {
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
  }, [spec.enumSource, desks, connections]);

  const branchOptions = useMemo(
    () => graph.nodes.map((n) => ({ id: n.id, label: `#${n.id} · ${nodeSummary(n, bookName)}` })),
    [graph.nodes, bookName],
  );

  return (
    <>
      <div className={styles.editorField}>
        <span className={styles.fieldLabel}>Field</span>
        <p className={styles.fieldStatic}>
          {spec.label} <span className={styles.kindPill}>{spec.kind}</span>
        </p>
        <span className={styles.fieldHint}>Drop another field chip on this card to rebind it.</span>
      </div>

      <label className={styles.editorField}>
        <span className={styles.fieldLabel}>Operator</span>
        <select
          className={styles.select}
          value={node.condition.op}
          disabled={readOnly}
          onChange={(e) => onSetOperator(node.id, e.target.value as RouteOp)}
          data-testid="op-select"
        >
          {spec.validOps.map((op) => (
            <option key={op} value={op}>
              {opLabel(op)}
            </option>
          ))}
        </select>
      </label>

      <ValueEditor
        op={node.condition.op}
        value={node.condition.value}
        kind={spec.kind}
        enumOptions={enumOptions}
        readOnly={readOnly}
        onChange={(v) => onSetValue(node.id, v)}
      />

      <div className={styles.branchRow}>
        <label className={styles.editorField}>
          <span className={`${styles.fieldLabel} ${styles.yesLabel}`}>If true → yes</span>
          <select
            className={styles.select}
            value={node.condition.onTrue}
            disabled={readOnly}
            onChange={(e) => onSetBranch(node.id, "onTrue", Number(e.target.value))}
            data-testid="branch-true-select"
          >
            {branchOptions.map((o) => (
              <option key={o.id} value={o.id}>
                {o.label}
              </option>
            ))}
          </select>
        </label>
        <label className={styles.editorField}>
          <span className={`${styles.fieldLabel} ${styles.noLabel}`}>If false → no</span>
          <select
            className={styles.select}
            value={node.condition.onFalse}
            disabled={readOnly}
            onChange={(e) => onSetBranch(node.id, "onFalse", Number(e.target.value))}
            data-testid="branch-false-select"
          >
            {branchOptions.map((o) => (
              <option key={o.id} value={o.id}>
                {o.label}
              </option>
            ))}
          </select>
        </label>
      </div>
    </>
  );
}

// --- Typed value editor ----------------------------------------------------

function ValueEditor({
  op,
  value,
  kind,
  enumOptions,
  readOnly,
  onChange,
}: {
  op: RouteOp;
  value: RouteValue | null;
  kind: "enum" | "numeric" | "string";
  enumOptions: Opt[];
  readOnly: boolean;
  onChange: (v: RouteValue) => void;
}): React.ReactElement {
  // `between` — two numeric bounds.
  if (op === "between") {
    const lo = value?.kind === "range" ? value.lo : 0;
    const hi = value?.kind === "range" ? value.hi : 0;
    return (
      <div className={styles.editorField}>
        <span className={styles.fieldLabel}>Range (inclusive)</span>
        <div className={styles.rangeRow}>
          <input
            className={styles.input}
            type="number"
            value={lo}
            disabled={readOnly}
            aria-label="Lower bound"
            onChange={(e) => onChange({ kind: "range", lo: Number(e.target.value), hi })}
          />
          <span className={styles.rangeSep}>to</span>
          <input
            className={styles.input}
            type="number"
            value={hi}
            disabled={readOnly}
            aria-label="Upper bound"
            onChange={(e) => onChange({ kind: "range", lo, hi: Number(e.target.value) })}
          />
        </div>
      </div>
    );
  }

  // `in` — membership. Enum source ⇒ toggle chips; else a comma/enter tag input.
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
        <input
          className={styles.input}
          type="number"
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
  enumOptions: Opt[];
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
              <button type="button" className={styles.tagX} aria-label={`Remove ${v}`} onClick={() => remove(v)}>
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
