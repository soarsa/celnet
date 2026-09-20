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
import { isDeskModal, getDeskModalAuthToken } from "../lib/deskPlatform";
import styles from "./SignInDialog.module.css";

export function SignInDialog(): React.ReactElement | null {
  const app = useApp();
  const { auth, signInOpen, setSignInOpen } = app;
  const titleId = useId();
  const emailRef = useRef<HTMLInputElement | null>(null);
  // `auth.clearError` is a stable useCallback; capturing it (rather than the whole
  // `auth` object, which `useAuth` rebuilds every render) keeps the reset effect
  // below from re-firing on unrelated app re-renders (e.g. stream ticks) and
  // wiping the user's keystrokes mid-type.
  const clearError = auth.clearError;

  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");

  // Reset the form (and clear any prior error) only on the open transition.
  useEffect(() => {
    if (!signInOpen) return;
    setEmail("");
    setPassword("");
    clearError();
  }, [signInOpen, clearError]);

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

  const inDesk = isDeskModal() || (typeof window !== "undefined" && window.location.protocol === "deskmodal-plugin:");
  const canSubmit = email.trim().length > 0 && password.length > 0 && !auth.busy;

  const handleHostSSO = async (): Promise<void> => {
    try {
      const token = await getDeskModalAuthToken();
      if (!token) {
        throw new Error("No active session found in DeskModal host container.");
      }
      await auth.loginWithToken(token);
      setSignInOpen(false);
    } catch (e) {
      console.warn("[CelNet] DeskModal SSO failed:", e);
    }
  };

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
            {inDesk ? "DeskModal Session & Identity" : "Sign in"}
          </h2>
          <p className={styles.sub}>
            {inDesk
              ? "Authentication and role claims are managed by the DeskModal desktop agent and passed through to CelNet services."
              : "Authenticate to administer users and desks, and to see your desk's inbound RFQ traffic."}
          </p>
        </div>

        {inDesk ? (
          <div style={{ display: "flex", flexDirection: "column", gap: "16px", marginTop: "12px" }}>
            {auth.user ? (
              <div
                style={{
                  background: "var(--color-bg-subtle, rgba(255, 255, 255, 0.05))",
                  border: "1px solid var(--color-border, rgba(255, 255, 255, 0.1))",
                  borderRadius: "8px",
                  padding: "16px",
                  display: "flex",
                  flexDirection: "column",
                  gap: "8px",
                }}
              >
                <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
                  <span style={{ fontSize: "12px", color: "var(--color-text-muted, #888)", textTransform: "uppercase" }}>
                    Authenticated Principal
                  </span>
                  <span
                    style={{
                      background: "rgba(16, 185, 129, 0.2)",
                      color: "#10b981",
                      borderRadius: "4px",
                      padding: "2px 8px",
                      fontSize: "11px",
                      fontWeight: 600,
                    }}
                  >
                    Active SSO
                  </span>
                </div>
                <div style={{ fontSize: "15px", fontWeight: 600, color: "var(--color-text-primary, #fff)" }}>
                  {auth.user.email}
                </div>
                <div style={{ fontSize: "12px", color: "var(--color-text-secondary, #bbb)" }}>
                  Role: <strong style={{ color: "var(--accent, #ff7357)" }}>{auth.user.role}</strong>
                  {auth.user.deskIds.length > 0 && ` · Desks: ${auth.user.deskIds.join(", ")}`}
                </div>
              </div>
            ) : (
              <div
                style={{
                  background: "var(--color-bg-subtle, rgba(255, 255, 255, 0.05))",
                  border: "1px solid var(--color-border, rgba(255, 255, 255, 0.1))",
                  borderRadius: "8px",
                  padding: "16px",
                  textAlign: "center",
                }}
              >
                <p style={{ fontSize: "13px", color: "var(--color-text-secondary, #aaa)", margin: "0 0 12px 0" }}>
                  {auth.busy
                    ? "Connecting to CelNet trading edge via DeskModal session token…"
                    : "Zero-credential authentication: your session token will be acquired directly from the DeskModal container."}
                </p>
                <Button
                  type="button"
                  variant="primary"
                  disabled={auth.busy}
                  onClick={() => void handleHostSSO()}
                >
                  {auth.busy ? "Verifying Host Session…" : "Authenticate via DeskModal SSO"}
                </Button>
              </div>
            )}

            {auth.error && (
              <p className={styles.error} role="alert">
                {auth.error}
              </p>
            )}

            <div className={styles.foot}>
              <Button type="button" variant="ghost" onClick={() => setSignInOpen(false)}>
                Close
              </Button>
              {auth.user && (
                <Button
                  type="button"
                  variant="primary"
                  disabled={auth.busy}
                  onClick={() => void handleHostSSO()}
                >
                  Refresh Host Session
                </Button>
              )}
            </div>
          </div>
        ) : (
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
        )}
      </div>
    </div>
  );
}
