/**
 * ScopeBreadcrumb — the toolbar affordance for "what slice of the firm am I
 * looking at" (P0-6). It renders the active `ScopeContext.path` as a breadcrumb
 * (Firm · Desk · Book · …); each ancestor crumb is clickable to drill back up. The
 * tail crumb is the current scope and is inert.
 *
 * Today the principal is `grant-all`, so the scope filters NOTHING — but every
 * data path already flows through it, so a real entitlement predicate slots in
 * later with zero rework (AppContext.ScopeContext). When the path is just the
 * firm root we append an honest descriptor of the implied span ("all desks · all
 * books") rather than leaving the crumb bare. Pair selection (PairMenu) is the
 * terminal scope case and lives to the right of this control.
 */

import { useApp } from "../app/AppContext";
import styles from "./ScopeBreadcrumb.module.css";

export function ScopeBreadcrumb(): React.ReactElement {
  const app = useApp();
  const { path } = app.scope;

  return (
    <nav className={styles.root} aria-label="scope">
      {path.map((node, i) => {
        const isLast = i === path.length - 1;
        return (
          <span key={`${node.level}:${node.label}`} className={styles.crumbWrap}>
            {i > 0 && (
              <span className={styles.sep} aria-hidden>
                ·
              </span>
            )}
            {isLast ? (
              <span className={styles.current} aria-current="true">
                {node.label}
              </span>
            ) : (
              <button
                type="button"
                className={styles.crumb}
                // Drill back up to this ancestor: truncate the path to it.
                onClick={() => app.setScopePath(path.slice(0, i + 1))}
                title={`Scope to ${node.label}`}
              >
                {node.label}
              </button>
            )}
          </span>
        );
      })}
      {path.length === 1 && (
        <span className={styles.implied} title="Grant-all: every desk, book and pair">
          all desks · all books
        </span>
      )}
    </nav>
  );
}
