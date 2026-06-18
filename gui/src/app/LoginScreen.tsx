/**
 * LoginScreen — the in-app sign-in surface the trader lands on after the backend
 * connection stays down past the reconnect window. It is intentionally minimal:
 * the celnet wire carries no identity (real authentication — mTLS / an
 * authenticating gateway — is a deployment-layer concern, mirroring the server's
 * entitlement boundary), so this form does not itself authenticate. Submitting it
 * re-enters the app, which re-dials the WS edge from a clean state.
 *
 * Branded with the Celer lockup (the mark + "Celnet" wordmark) — the same lockup
 * the design system reserves for splash/about/login surfaces.
 */

import { useState } from "react";
import { CelerLockup } from "../components/CelerMark";
import styles from "./LoginScreen.module.css";

export interface LoginScreenProps {
  /**
   * Invoked when the trader signs in. The app re-establishes the session from a
   * clean state (a full reload re-dials the configured WS edge), so a recovered
   * backend is picked up without any stale socket/subscription state.
   */
  readonly onSignIn: () => void;
  /** Optional note explaining why the trader was returned here (e.g. the lost endpoint). */
  readonly reason?: string;
}

export function LoginScreen({ onSignIn, reason }: LoginScreenProps): React.ReactElement {
  const [seat, setSeat] = useState("");

  const handleSubmit = (event: React.FormEvent<HTMLFormElement>): void => {
    event.preventDefault();
    onSignIn();
  };

  return (
    <main className={styles.screen}>
      <section className={styles.panel} aria-labelledby="login-heading">
        <CelerLockup size={28} className={styles.lockup} />
        <h1 id="login-heading" className={styles.heading}>
          Sign in to continue
        </h1>
        <p className={styles.sub}>
          {reason ?? "Your session ended after the connection to the trading edge was lost."}
        </p>

        <form className={styles.form} onSubmit={handleSubmit}>
          <label className={styles.field}>
            <span className={styles.label}>Seat</span>
            <input
              className={styles.input}
              type="text"
              name="seat"
              autoComplete="username"
              placeholder="desk / trader id"
              value={seat}
              onChange={(e) => setSeat(e.target.value)}
              autoFocus
            />
          </label>
          <label className={styles.field}>
            <span className={styles.label}>Passcode</span>
            <input
              className={styles.input}
              type="password"
              name="passcode"
              autoComplete="current-password"
              placeholder="••••••••"
            />
          </label>
          <button type="submit" className={styles.submit}>
            Sign in
          </button>
        </form>

        <p className={styles.footnote}>
          Authentication is enforced at the deployment edge; this screen re-opens the
          session against the configured trading edge.
        </p>
      </section>
    </main>
  );
}
