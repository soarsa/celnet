/**
 * UpdateBanner — a slim, non-blocking attention strip shown when a newer release
 * has been deployed under the running page (detected by {@link useVersionWatch}).
 *
 * Deliberately NOT auto-reloading: this is a trading surface, and silently
 * swapping the bundle mid-ticket would discard in-progress work. The trader
 * chooses when to reload; the banner can be dismissed until the next release.
 *
 * Pure presentation — all detection/latch state lives in the version watcher and
 * the App root that owns the dismiss decision.
 */

import type { ReleaseManifest } from "../data/versionManifest";
import styles from "./UpdateBanner.module.css";

export interface UpdateBannerProps {
  /** The newer release the page can reload onto. */
  readonly release: ReleaseManifest;
  /** Reload the page onto the new bundle (App passes `window.location.reload`). */
  readonly onReload: () => void;
  /** Hide the banner until an even newer release appears. */
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
  return (
    <div className={styles.banner} role="status" aria-live="polite">
      <span className={styles.glyph} aria-hidden="true">
        ⟳
      </span>
      <span className={styles.text}>
        New release available
        <span className={styles.detail}>
          {release.hash} · {shortStamp(release.buildTime)}
        </span>
      </span>
      <button type="button" className={styles.reload} onClick={onReload}>
        Reload
      </button>
      <button
        type="button"
        className={styles.dismiss}
        onClick={onDismiss}
        aria-label="Dismiss until the next release"
      >
        ×
      </button>
    </div>
  );
}
