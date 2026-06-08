/**
 * SavedViewsMenu — the title-bar control for saved views (GW1-S4). A saved view is
 * the reproducible triple {active workspace, scope drill path + group-by, inspector
 * analytics}: the URL is the canonical form (a link IS the view) and localStorage
 * mirrors the named set, so any (scope × view × analytics) is bookmarkable.
 *
 * The menu: name + save the CURRENT view; recall or delete a saved one; copy the
 * share link (the current view's canonical URL). Closes on select / Escape /
 * outside click — the same anchored-dropdown grammar the deleted PairMenu used,
 * now serving the ONE remaining drop-down concern (saved views), not pair switching.
 */

import { useCallback, useEffect, useId, useRef, useState } from "react";
import { useApp } from "../app/AppContext";
import { encodeViewString } from "../lib/savedViews";
import styles from "./SavedViewsMenu.module.css";

export function SavedViewsMenu(): React.ReactElement {
  const app = useApp();
  const [open, setOpen] = useState(false);
  const [name, setName] = useState("");
  const rootRef = useRef<HTMLDivElement | null>(null);
  const menuId = useId();

  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent): void => {
      if (rootRef.current && !rootRef.current.contains(e.target as Node)) setOpen(false);
    };
    const onKey = (e: KeyboardEvent): void => {
      if (e.key === "Escape") setOpen(false);
    };
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [open]);

  const save = useCallback((): void => {
    const trimmed = name.trim();
    if (trimmed.length === 0) return;
    app.saveView(trimmed);
    setName("");
  }, [app, name]);

  const shareLink = useCallback((): void => {
    if (typeof window === "undefined") return;
    const qs = encodeViewString(app.viewState);
    const url = `${window.location.origin}${window.location.pathname}${qs.length > 0 ? `?${qs}` : ""}`;
    // Best-effort clipboard copy (a no-op where the API is unavailable).
    void navigator.clipboard?.writeText(url).catch(() => {});
  }, [app.viewState]);

  return (
    <div className={styles.root} ref={rootRef}>
      <button
        type="button"
        className={styles.trigger}
        onClick={() => setOpen((o) => !o)}
        aria-haspopup="menu"
        aria-expanded={open}
        aria-controls={open ? menuId : undefined}
        title="Saved views"
      >
        <span className={styles.glyph} aria-hidden>
          ⌖
        </span>
        <span className={styles.label}>Views</span>
        <span className={`num ${styles.count}`}>{app.savedViews.length}</span>
      </button>

      {open && (
        <div className={styles.menu} id={menuId} role="menu" aria-label="saved views">
          <div className={styles.saveRow}>
            <input
              className={styles.nameInput}
              type="text"
              value={name}
              onChange={(e) => setName(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") save();
              }}
              placeholder="Name this view…"
              aria-label="name the current view"
              spellCheck={false}
              autoComplete="off"
            />
            <button
              type="button"
              className={styles.saveBtn}
              onClick={save}
              disabled={name.trim().length === 0}
            >
              Save
            </button>
          </div>

          <button type="button" role="menuitem" className={styles.shareItem} onClick={shareLink}>
            <span>Copy share link</span>
            <span className={styles.shareGlyph} aria-hidden>
              ⎘
            </span>
          </button>

          {app.savedViews.length === 0 ? (
            <p className={styles.empty}>No saved views yet.</p>
          ) : (
            <ul className={styles.list}>
              {app.savedViews.map((v) => (
                <li key={v.id} className={styles.item}>
                  <button
                    type="button"
                    role="menuitem"
                    className={styles.recall}
                    onClick={() => {
                      app.recallView(v.id);
                      setOpen(false);
                    }}
                    title={`Recall “${v.name}”`}
                  >
                    {v.name}
                  </button>
                  <button
                    type="button"
                    className={styles.del}
                    onClick={() => app.deleteView(v.id)}
                    aria-label={`delete saved view ${v.name}`}
                    title={`Delete “${v.name}”`}
                  >
                    <span aria-hidden>✕</span>
                  </button>
                </li>
              ))}
            </ul>
          )}
        </div>
      )}
    </div>
  );
}
