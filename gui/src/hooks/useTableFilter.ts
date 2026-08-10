/**
 * useTableFilter — the ONE reusable client-side row-filter primitive shared by
 * every tabular blotter (deals, quotes, positions, orders). Given the rows a
 * table already produces (in whatever order/grouping it chose) and a projection
 * from a row to its user-visible search text, it returns the case-insensitive
 * substring-matched subset plus the shown/total counts for a `<TableSearch>`.
 *
 * The filter sits ON TOP of the caller's ordering — it only ever removes rows,
 * never reorders — so existing sort/group semantics are preserved. An empty (or
 * whitespace-only) query passes every row through. The match is memoised on the
 * rows + query so unrelated re-renders (e.g. a sibling field changing) do not
 * re-scan; the projection is read through a ref so passing an inline arrow does
 * not defeat the memo.
 */

import { useMemo, useRef, useState } from "react";

/** The filter state + result surfaced to a table and its `<TableSearch>`. */
export type TableFilter<T> = {
  /** The current raw query text (what the user typed). */
  query: string;
  /** Set the query (wire straight to the search input's onChange). */
  setQuery: (query: string) => void;
  /** The rows that match the query, in the caller's original order. */
  filtered: T[];
  /** How many rows match (`filtered.length`) — the "N" in "N of M". */
  shown: number;
  /** How many rows there are in total (`rows.length`) — the "M" in "N of M". */
  total: number;
};

/**
 * Filter `rows` by a case-insensitive substring match of `query` against each
 * row's `toSearchText(row)` projection.
 *
 * @param rows the table's rows, already ordered/grouped by the caller
 * @param toSearchText projects a row to the concatenated text to match against
 *   (instrument, counterparty, side, state, tenor, id, …)
 * @param controlled an OPTIONAL externally-owned `[query, setQuery]` pair — pass a
 *   value from `useTableUiState` so the search text SURVIVES the workspace
 *   unmounting on a tab switch and is restored on return. Omit it (the default) to
 *   keep the query in the hook's own local state (the prior behaviour, unchanged).
 */
export function useTableFilter<T>(
  rows: readonly T[],
  toSearchText: (row: T) => string,
  controlled?: { query: string; setQuery: (query: string) => void },
): TableFilter<T> {
  const [localQuery, setLocalQuery] = useState("");
  const query = controlled ? controlled.query : localQuery;
  const setQuery = controlled ? controlled.setQuery : setLocalQuery;

  // Read the projection through a ref so an inline arrow at the call site does
  // not change the memo's dependency identity every render.
  const projectRef = useRef(toSearchText);
  projectRef.current = toSearchText;

  const filtered = useMemo(() => {
    const needle = query.trim().toLowerCase();
    if (needle === "") return rows.slice();
    const project = projectRef.current;
    return rows.filter((row) => project(row).toLowerCase().includes(needle));
  }, [rows, query]);

  return { query, setQuery, filtered, shown: filtered.length, total: rows.length };
}
