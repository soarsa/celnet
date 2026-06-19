/**
 * SignInDialog — the modal sign-in surface for server-enforced sessions.
 *
 * A scrim+blur modal (the FixConnectionWizard/ScopeSwitcher material) carrying an
 * email + password form that calls `AuthService.Login` through `app.auth.login`.
 * On success the bearer token is installed on the transport (see {@link useAuth})
 * and the dialog closes; a rejected login shows the server's message inline. It
 * is rendered ONCE in the Shell and driven by `app.signInOpen`, so any affordance
 * (the title-bar identity menu, the Admin workspace's gate) opens the same dialog.
 *
 * Accessibility: `role="dialog"` + `aria-modal`, a labelled title, Esc to close,
 * and initial focus on the email field.
 */

import { useEffect, useId, useRef, useState } from "react";

import { useApp } from "../app/AppContext";
import { CelerLockup } from "./CelerMark";
import { Button } from "./Button";
import styles from "./SignInDialog.module.css";

export function SignInDialog(): React.ReactElement | null {
  const app = useApp();
  const { auth, signInOpen, setSignInOpen } = app;
  const titleId = useId();
  const emailRef = useRef<HTMLInputElement | null>(null);

  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");

  // Reset the form (and clear any prior error) each time the dialog opens.
  useEffect(() => {
    if (!signInOpen) return;
    setEmail("");
    setPassword("");
    auth.clearError();
  }, [signInOpen, auth]);

  // Focus the email field on open.
  useEffect(() => {
    if (signInOpen) emailRef.current?.focus();
  }, [signInOpen]);

  // Esc closes the dialog while it is open.
  useEffect(() => {
    if (!signInOpen) return;
    const onKey = (e: KeyboardEvent): void => {
      if (e.key === "Escape") {
        e.preventDefault();
        setSignInOpen(false);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [signInOpen, setSignInOpen]);

  if (!signInOpen) return null;

  const canSubmit = email.trim().length > 0 && password.length > 0 && !auth.busy;

  const handleSubmit = (event: React.FormEvent<HTMLFormElement>): void => {
    event.preventDefault();
    if (!canSubmit) return;
    // On success close; on failure useAuth has set `auth.error` (shown below) —
    // swallow the rejection so the dialog stays open for a retry.
    void auth
      .login(email.trim(), password)
      .then(() => setSignInOpen(false))
      .catch(() => {});
  };

  return (
    <div
      className={styles.scrim}
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) setSignInOpen(false);
      }}
    >
      <div className={styles.panel} role="dialog" aria-modal="true" aria-labelledby={titleId}>
        <div className={styles.head}>
          <CelerLockup size={24} className={styles.lockup} />
          <h2 id={titleId} className={styles.title}>
            Sign in
          </h2>
          <p className={styles.sub}>
            Authenticate to administer users and desks, and to see your desk&apos;s inbound RFQ
            traffic.
          </p>
        </div>

        <form className={styles.form} onSubmit={handleSubmit}>
          <label className={styles.field}>
            <span className={styles.fieldLabel}>Email</span>
            <input
              ref={emailRef}
              className={styles.input}
              type="email"
              name="email"
              autoComplete="username"
              placeholder="admin@celnet.com"
              value={email}
              onChange={(e) => setEmail(e.target.value)}
            />
          </label>
          <label className={styles.field}>
            <span className={styles.fieldLabel}>Password</span>
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
          <div className={styles.foot}>
            <Button type="button" variant="ghost" onClick={() => setSignInOpen(false)}>
              Cancel
            </Button>
            <Button type="submit" variant="primary" disabled={!canSubmit}>
              {auth.busy ? "Signing in…" : "Sign in"}
            </Button>
          </div>
        </form>
      </div>
    </div>
  );
}
