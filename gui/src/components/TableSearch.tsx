/**
 * TableSearch — the shared presentational search box mounted above every
 * tabular blotter (deals, quotes, positions, orders). Purely controlled: it owns
 * no state, it renders the `query` it is given and reports edits via
 * `onQueryChange`; the matching itself lives in {@link useTableFilter}. It shows
 * an inline "N of M" count and, once the query is non-empty, a clear (×) button.
 *
 * Accessibility: the input carries an explicit `aria-label`, the clear button an
 * `aria-label` derived from it, and the count is an `aria-live` region so a
 * screen reader hears the result shrink as the trader types.
 */

import styles from "./TableSearch.module.css";

type Props = {
  /** The current query text (owned by the caller / `useTableFilter`). */
  query: string;
  /** Called with the new query on every keystroke and on clear. */
  onQueryChange: (query: string) => void;
  /** Matching row count — the "N" in "N of M". */
  shown: number;
  /** Total row count — the "M" in "N of M". */
  total: number;
  /** Accessible label for the input, e.g. "Search deals". */
  label: string;
  /** Placeholder text; defaults to "Search". */
  placeholder?: string;
};

export function TableSearch({
  query,
  onQueryChange,
  shown,
  total,
  label,
  placeholder = "Search",
}: Props) {
  const hasQuery = query.trim() !== "";

  return (
    <div className={styles.wrap}>
      <span className={styles.field}>
        <span className={styles.glyph} aria-hidden="true">
          ⌕
        </span>
        <input
          type="search"
          className={styles.input}
          value={query}
          aria-label={label}
          placeholder={placeholder}
          spellCheck={false}
          autoComplete="off"
          onChange={(e) => onQueryChange(e.target.value)}
        />
        {hasQuery && (
          <button
            type="button"
            className={styles.clear}
            aria-label={`Clear ${label}`}
            onClick={() => onQueryChange("")}
          >
            ×
          </button>
        )}
      </span>
      <span className={styles.count} aria-live="polite">
        {shown} of {total}
      </span>
    </div>
  );
}
