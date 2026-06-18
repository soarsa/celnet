/**
 * ReconnectOverlay — the full-screen blocking modal shown while the backend
 * connection is down and the transport is reconnecting. It covers the (now stale)
 * workspace, surfaces a live countdown of the reconnect window, and offers an
 * immediate escape to the sign-in screen. When the window elapses the parent
 * (`App`) drops to `LoginScreen`; until then this overlay holds.
 *
 * Pure presentation: all timing/state lives in `useConnectionStatus`.
 */

import { CelerMark } from "../components/CelerMark";
import styles from "./ReconnectOverlay.module.css";

export interface ReconnectOverlayProps {
  /** Whole seconds left before the app redirects to sign-in. */
  readonly remainingSeconds: number;
  /** The transport endpoint label (e.g. "live ws://127.0.0.1:8081"). */
  readonly endpointLabel: string;
  /** Skip the countdown and go to the sign-in screen now. */
  readonly onSignInNow: () => void;
}

export function ReconnectOverlay({
  remainingSeconds,
  endpointLabel,
  onSignInNow,
}: ReconnectOverlayProps): React.ReactElement {
  const seconds = Math.max(0, remainingSeconds);
  return (
    <div
      className={styles.scrim}
      role="alertdialog"
      aria-modal="true"
      aria-labelledby="reconnect-title"
      aria-describedby="reconnect-body"
    >
      <div className={styles.card}>
        <div className={styles.markRing} aria-hidden="true">
          <CelerMark size={36} className={styles.mark} />
        </div>
        <h2 id="reconnect-title" className={styles.title}>
          Connection lost
        </h2>
        <p id="reconnect-body" className={styles.body}>
          Reconnecting to <span className={styles.endpoint}>{endpointLabel}</span>…
        </p>
        <p className={styles.countdown} aria-live="polite">
          Returning to sign-in in <span className={styles.count}>{seconds}s</span>
        </p>
        <button type="button" className={styles.signInNow} onClick={onSignInNow}>
          Sign in now
        </button>
      </div>
    </div>
  );
}
