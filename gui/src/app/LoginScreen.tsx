/**
 * LoginScreen — the in-app sign-in surface the trader lands on after the backend
 * connection stays down past the reconnect window. It is intentionally minimal:
 * the celnet wire carries no identity (real authentication — mTLS / an
 * authenticating gateway — is a deployment-layer concern, mirroring the server's
 * entitlement boundary), so this form does not itself authenticate. Submitting it
 * re-enters the app, which re-dials the WS edge from a clean state.
 *
 * Branded with the Celnet lockup (the mark + "Celnet" wordmark) — the same lockup
 * the design system reserves for splash/about/login surfaces.
 */

import { CelerLockup } from "../components/CelerMark";
import { isDeskModal } from "../lib/deskPlatform";
import styles from "./LoginScreen.module.css";

export interface LoginScreenProps {
  /**
   * Invoked when the trader reconnects. The app re-establishes the session from a
   * clean state (a full reload re-dials the configured WS edge), so a recovered
   * backend is picked up without any stale socket/subscription state.
   */
  readonly onSignIn: () => void;
  /** Optional note explaining why the trader was returned here (e.g. the lost endpoint). */
  readonly reason?: string;
}

export function LoginScreen({ onSignIn, reason }: LoginScreenProps): React.ReactElement {
  const inDesk = isDeskModal() || (typeof window !== "undefined" && window.location.protocol === "deskmodal-plugin:");

  const handleSubmit = (event: React.FormEvent<HTMLFormElement>): void => {
    event.preventDefault();
    onSignIn();
  };

  return (
    <main className={styles.screen}>
      <section className={styles.panel} aria-labelledby="login-heading">
        <CelerLockup size={28} className={styles.lockup} />
        <h1 id="login-heading" className={styles.heading}>
          {inDesk ? "Trading Edge Disconnected" : "Session Disconnected"}
        </h1>
        <p className={styles.sub}>
          {reason ?? "Your session ended after the connection to the trading edge was lost."}
        </p>

        <form className={styles.form} onSubmit={handleSubmit}>
          <button type="submit" className={styles.submit}>
            {inDesk ? "Reconnect via DeskModal SSO" : "Reconnect to Trading Edge"}
          </button>
        </form>

        <p className={styles.footnote}>
          {inDesk
            ? "Authentication and identity claims are externalized to DeskModal and passed through to CelNet services."
            : "Authentication is enforced at the deployment edge or enterprise gateway; reconnecting re-establishes the session."}
        </p>
      </section>
    </main>
  );
}
