/**
 * LoginView — the mandatory sign-in surface shown as the whole page until a user
 * authenticates. Server-enforced sessions are required: the workspace is not
 * reachable without signing in (the `AuthGate` renders this instead of the Shell
 * whenever `auth.user` is null).
 *
 * It calls `AuthService.Login` through `app.auth.login`; on success the bearer
 * token is installed on the transport and the gate re-renders into the Shell. A
 * rejected login shows the server's message inline.
 *
 * The inputs are plain controlled fields with NO reset-on-render effect, so app
 * re-renders (e.g. the price stream ticking under the provider) never wipe what
 * the operator is typing.
 */

import { useState } from "react";

import { useApp } from "./AppContext";
import { CelerLockup } from "../components/CelerMark";
import { isDeskModal } from "../lib/deskPlatform";
import styles from "./LoginScreen.module.css";

export function LoginView(): React.ReactElement {
  const { auth } = useApp();
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");

  const inDesk = isDeskModal() || (typeof window !== "undefined" && window.location.protocol === "deskmodal-plugin:");

  const canSubmit = email.trim().length > 0 && password.length > 0 && !auth.busy;

  const handleSubmit = (event: React.FormEvent<HTMLFormElement>): void => {
    event.preventDefault();
    if (!canSubmit) return;
    // useAuth sets `auth.error` on failure (shown below); swallow the rejection so
    // the form stays put for a retry.
    void auth.login(email.trim(), password).catch(() => {});
  };

  if (inDesk) {
    return (
      <main className={styles.screen}>
        <section className={styles.panel} aria-labelledby="login-heading">
          <CelerLockup size={28} className={styles.lockup} />
          <h1 id="login-heading" className={styles.heading}>
            DeskModal SSO Pass-Through
          </h1>
          <p className={styles.sub}>
            {auth.busy
              ? "Connecting to CelNet trading edge via DeskModal session token…"
              : auth.error
                ? auth.error
                : "Awaiting DeskModal desktop container authorization…"}
          </p>

          {auth.busy && (
            <div style={{ margin: "20px 0", textAlign: "center" }}>
              <div
                style={{
                  display: "inline-block",
                  padding: "8px 16px",
                  borderRadius: "6px",
                  background: "var(--color-bg-subtle, rgba(255, 255, 255, 0.06))",
                  fontSize: "12px",
                  color: "var(--color-text-muted, #999)",
                  letterSpacing: "0.04em",
                  textTransform: "uppercase",
                }}
              >
                Verifying Cryptographic Host Token…
              </div>
            </div>
          )}

          {auth.error && (
            <button
              type="button"
              className={styles.submit}
              onClick={() => {
                auth.clearError();
                window.location.reload();
              }}
            >
              Retry DeskModal Session Handshake
            </button>
          )}

          <p className={styles.footnote}>
            Zero-credential architecture: authentication is managed by DeskModal and passed through to CelNet services.
          </p>
        </section>
      </main>
    );
  }

  return (
    <main className={styles.screen}>
      <section className={styles.panel} aria-labelledby="login-heading">
        <CelerLockup size={28} className={styles.lockup} />
        <h1 id="login-heading" className={styles.heading}>
          Sign in to Celnet
        </h1>
        <p className={styles.sub}>Authenticate to access the trading platform.</p>

        <form className={styles.form} onSubmit={handleSubmit}>
          <label className={styles.field}>
            <span className={styles.label}>Email</span>
            <input
              className={styles.input}
              type="email"
              name="email"
              autoComplete="username"
              placeholder="admin@celnet.com"
              value={email}
              onChange={(e) => setEmail(e.target.value)}
              autoFocus
            />
          </label>
          <label className={styles.field}>
            <span className={styles.label}>Password</span>
            <input
              className={styles.input}
              type="password"
              name="password"
              autoComplete="current-password"
              placeholder="••••••••"
              value={password}
              onChange={(e) => setPassword(e.target.value)}
            />
          </label>
          {auth.error && (
            <p className={styles.error} role="alert">
              {auth.error}
            </p>
          )}
          <button type="submit" className={styles.submit} disabled={!canSubmit}>
            {auth.busy ? "Signing in…" : "Sign in"}
          </button>
        </form>

        <p className={styles.footnote}>
          Server-enforced session. Access is scoped to your role and desk.
        </p>
      </section>
    </main>
  );
}
