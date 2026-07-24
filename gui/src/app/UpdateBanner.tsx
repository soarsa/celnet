/**
 * UpdateBanner — a blocking-style **modal** shown when a newer release has been
 * deployed under the running page (detected by {@link useVersionWatch}). It tells
 * the trader a new version is available and prompts them to refresh.
 *
 * Deliberately NOT auto-reloading: this is a trading surface, and silently
 * swapping the bundle mid-ticket would discard in-progress work. The modal makes
 * the update prominent (centred, over a scrim) but the trader still chooses when
 * to refresh — "Refresh now" reloads immediately, "Later" dismisses until an even
 * newer release appears.
 *
 * Pure presentation — all detection/latch state lives in the version watcher and
 * the App root that owns the dismiss decision. Rendered via a portal to
 * `document.body` so it escapes any workspace overflow/stacking context.
 */

import { useEffect, useRef } from "react";
import { createPortal } from "react-dom";

import type { ReleaseManifest } from "../data/versionManifest";
import styles from "./UpdateBanner.module.css";

export interface UpdateBannerProps {
  /** The newer release the page can reload onto. */
  readonly release: ReleaseManifest;
  /** Reload the page onto the new bundle (App passes `window.location.reload`). */
  readonly onReload: () => void;
  /** Hide the modal until an even newer release appears. */
  readonly onDismiss: () => void;
}

/** `2026-06-27T13:25:28.000Z` -> `2026-06-27 13:25Z` for a compact label. */
function shortStamp(iso: string): string {
  const m = /^(\d{4}-\d{2}-\d{2})T(\d{2}:\d{2})/.exec(iso);
  return m ? `${m[1]} ${m[2]}Z` : iso;
}

export function UpdateBanner({
  release,
  onReload,
  onDismiss,
}: UpdateBannerProps): React.ReactElement {
  const reloadRef = useRef<HTMLButtonElement>(null);

  // Move focus to the primary action on open, and let Esc dismiss (matching the
  // app's other portal dialogs).
  useEffect(() => {
    reloadRef.current?.focus();
    const onKey = (e: KeyboardEvent): void => {
      if (e.key === "Escape") onDismiss();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onDismiss]);

  return createPortal(
    <div
      className={styles.scrim}
      role="presentation"
      onClick={(e) => {
        if (e.target === e.currentTarget) onDismiss();
      }}
    >
      <div
        className={styles.dialog}
        role="dialog"
        aria-modal="true"
        aria-labelledby="update-modal-title"
      >
        <div className={styles.head}>
          <span className={styles.glyph} aria-hidden="true">
            ⟳
          </span>
          <h2 id="update-modal-title" className={styles.title}>
            New version available
          </h2>
        </div>
        <p className={styles.body}>
          A new version of Celnet has been deployed. Refresh the screen to load it — your
          current view will reload.
        </p>
        <p className={styles.detail}>
          {release.hash} · {shortStamp(release.buildTime)}
        </p>
        <div className={styles.actions}>
          <button
            type="button"
            className={styles.later}
            onClick={onDismiss}
            aria-label="Dismiss until the next release"
          >
            Later
          </button>
          <button
            type="button"
            ref={reloadRef}
            className={styles.reload}
            onClick={onReload}
          >
            Refresh now
          </button>
        </div>
      </div>
    </div>,
    document.body,
  );
}
