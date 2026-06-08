/**
 * ScopeControl — the ONE breadcrumb-driven scope control (GW1-S2/S3).
 *
 * This is the single affordance for "what slice of the firm am I looking at",
 * and its TERMINAL case is underlier selection. It REPLACES the drill-up-only
 * `ScopeBreadcrumb` and ABSORBS the four-plus redundant pair affordances
 * (`PairMenu`, the "Pairs" button, the `⌘K` pair list, `PairStrip`, the
 * `UniverseNavigator` overlay): there is now exactly one place to change scope.
 *
 * Grammar (reads `lib/scope.ts` via AppContext):
 *   • each ANCESTOR crumb is a button that drills UP (truncates the path to it);
 *   • a "drill ⌄" button drills DOWN one ladder level (firm→desk→book→pair), via
 *     the scope switcher (the re-homed Universe) so the trader picks the child;
 *   • the TERMINAL pair crumb is a button that opens the switcher's pair-universe
 *     leaf — underlier selection — and tracks the active pair (FX terminal == pair);
 *   • a group-by control pins the secondary aggregation axis (order-independent).
 *
 * Entitlement-ready: the principal is `grant-all` today (filters nothing), but the
 * scope flows through every data path so a real predicate slots in with zero rework.
 */

import { useApp } from "../app/AppContext";
import { childLevel, currentLevel, SCOPE_GROUP_BY, type ScopeGroupBy } from "../lib/scope";
import styles from "./ScopeControl.module.css";

/** Human label for a group-by axis (the menu options). */
const GROUP_BY_LABEL: Record<ScopeGroupBy, string> = {
  none: "No grouping",
  desk: "by Desk",
  book: "by Book",
  pair: "by Pair",
};

/** The next ladder level's noun, for the drill-down button label. */
const CHILD_NOUN: Record<string, string> = {
  desk: "desk",
  book: "book",
  pair: "pair",
};

export function ScopeControl(): React.ReactElement {
  const app = useApp();
  const { path } = app.scope;
  const tailLevel = currentLevel(app.scope);
  const next = childLevel(tailLevel);

  return (
    <div className={styles.root}>
      <nav className={styles.crumbs} aria-label="scope">
        {path.map((node, i) => {
          const isLast = i === path.length - 1;
          const isPairCrumb = node.level === "pair";
          return (
            <span key={`${node.level}:${i}`} className={styles.crumbWrap}>
              {i > 0 && (
                <span className={styles.sep} aria-hidden>
                  ›
                </span>
              )}
              {isLast ? (
                // The current scope. A PAIR terminal crumb is interactive (opens the
                // switcher to re-select the underlier); a non-pair terminal is inert.
                isPairCrumb ? (
                  <button
                    type="button"
                    className={`${styles.current} ${styles.currentButton}`}
                    aria-current="true"
                    onClick={() => app.setScopeSwitcherOpen(true)}
                    title="Change underlier"
                  >
                    {node.label}
                  </button>
                ) : (
                  <span className={styles.current} aria-current="true">
                    {node.label}
                  </span>
                )
              ) : (
                <button
                  type="button"
                  className={styles.crumb}
                  // Drill back UP to this ancestor: truncate the path to it.
                  onClick={() => app.drillScopeUp(i + 1)}
                  title={`Scope to ${node.label}`}
                >
                  {node.label}
                </button>
              )}
            </span>
          );
        })}

        {/* Drill DOWN one level — opens the scope switcher to pick the child node
            (an org node for desk/book; a pair at the terminal level). */}
        {next !== null && (
          <button
            type="button"
            className={styles.drill}
            onClick={() => app.setScopeSwitcherOpen(true)}
            title={`Drill into a ${CHILD_NOUN[next]}`}
            aria-label={`drill into a ${CHILD_NOUN[next]}`}
          >
            <span aria-hidden>⌄</span>
            <span className={styles.drillLabel}>{CHILD_NOUN[next]}</span>
          </button>
        )}

        {path.length === 1 && (
          <span className={styles.implied} title="Grant-all: every desk, book and pair">
            all desks · all books
          </span>
        )}
      </nav>

      {/* Secondary group-by axis (pins independently of the drill path). */}
      <label className={styles.groupBy}>
        <span className={styles.groupByLabel}>Group</span>
        <select
          className={styles.groupBySelect}
          value={app.scope.groupBy}
          onChange={(e) => app.setScopeGroupBy(e.target.value as ScopeGroupBy)}
          aria-label="group by"
        >
          {SCOPE_GROUP_BY.map((g) => (
            <option key={g} value={g}>
              {GROUP_BY_LABEL[g]}
            </option>
          ))}
        </select>
      </label>
    </div>
  );
}
