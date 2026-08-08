/**
 * UpdateBanner — the auto-refresh notice shown when a newer release has been
 * deployed under the running page (detected by {@link useVersionWatch}). Unlike the
 * old "please refresh" prompt, this does NOT leave the trader stranded on a stale
 * bundle: it announces the update and then performs a cache-busting reload onto the
 * fresh build after a short countdown, with a "Reload now" button for the impatient.
 *
 * Why auto-reload (a reversal of the earlier prompt-only stance): a plain "there's
 * an update" prompt that the trader dismisses — or a prompt whose reload does not
 * bust the browser cache — leaves them running the old app while believing the
 * deploy landed. The single most reliable behaviour is to refetch the (content-
 * hashed, cache-busted) bundle automatically. The countdown gives a brief, visible
 * grace window rather than yanking the view instantly.
 *
 * The actual cache-clear + reload lives in {@link resetAndReloadTo} (passed in as
 * `onReload`) and is guarded to fire at most ONCE per detected build — so this
 * banner can never drive a reload-loop. This component only owns the countdown UI
 * and calls `onReload` exactly once (a fired-latch ref).
 *
 * Rendered via a portal to `document.body` (escapes workspace overflow/stacking) as
 * a non-dismissable `role="status"` live region so assistive tech announces it once.
 */

import { useCallback, useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";

import type { ReleaseManifest } from "../data/versionManifest";
import styles from "./UpdateBanner.module.css";

/** Seconds of visible grace before the automatic reload fires. */
export const DEFAULT_COUNTDOWN_SECONDS = 5;

export interface UpdateBannerProps {
  /** The newer release the page is reloading onto. */
  readonly release: ReleaseManifest;
  /**
   * Perform the cache-busting reload onto {@link release}. App passes
   * `resetAndReloadTo(release)`; it is idempotent per build, so calling it more than
   * once is harmless — but this component still fires it at most once.
   */
  readonly onReload: () => void;
  /** Grace seconds before the auto-reload. Defaults to {@link DEFAULT_COUNTDOWN_SECONDS}. */
  readonly countdownSeconds?: number;
}

export function UpdateBanner({
  release,
  onReload,
  countdownSeconds = DEFAULT_COUNTDOWN_SECONDS,
}: UpdateBannerProps): React.ReactElement {
  const [remaining, setRemaining] = useState(countdownSeconds);
  const reloadRef = useRef<HTMLButtonElement>(null);
  // Latest onReload without resetting the countdown timers when the parent re-renders
  // with a fresh closure.
  const onReloadRef = useRef(onReload);
  onReloadRef.current = onReload;
  // Fire the reload at most once (manual button OR countdown, whichever first).
  const firedRef = useRef(false);

  const triggerReload = useCallback((): void => {
    if (firedRef.current) return;
    firedRef.current = true;
    onReloadRef.current();
  }, []);

  // A NEW target release (an even newer build landed mid-countdown) restarts the
  // grace window and re-arms the fired-latch, and moves focus to the action.
  useEffect(() => {
    firedRef.current = false;
    setRemaining(countdownSeconds);
    reloadRef.current?.focus();
  }, [release.buildTime, countdownSeconds]);

  // One interval ticks the grace window down to zero (restarted for a new build).
  useEffect(() => {
    const id = setInterval(() => setRemaining((r) => (r <= 0 ? 0 : r - 1)), 1000);
    return () => clearInterval(id);
  }, [release.buildTime, countdownSeconds]);

  // Fire the (guarded, once-only) reload when the countdown reaches zero.
  useEffect(() => {
    if (remaining <= 0) triggerReload();
  }, [remaining, triggerReload]);

  return createPortal(
    <div
      className={styles.banner}
      role="status"
      aria-live="polite"
      aria-label="Updating to the latest version"
      data-testid="update-banner"
    >
      <span className={styles.glyph} aria-hidden="true">
        ⟳
      </span>
      <div className={styles.text}>
        <p className={styles.title}>Updating to the latest version…</p>
        <p className={styles.detail}>
          A new build was deployed — reloading shortly{" "}
          <span aria-hidden="true">({remaining}s)</span>.
        </p>
      </div>
      <button
        type="button"
        ref={reloadRef}
        className={styles.reload}
        onClick={triggerReload}
      >
        Reload now
      </button>
    </div>,
    document.body,
  );
}
