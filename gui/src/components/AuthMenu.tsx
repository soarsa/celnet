/**
 * AuthMenu — the title-bar identity affordance for server-enforced sessions.
 *
 * Anonymous: a "Sign in" button that opens the shared {@link SignInDialog}
 * (`app.setSignInOpen(true)`). Signed in: the user's email + role, with a
 * "Sign out" action that invalidates the bearer token (`app.auth.logout`). The
 * identity here drives what the rest of the app sees — admin RPCs and the
 * desk-scoped FIX monitor key off the installed session.
 */

import { useApp } from "../app/AppContext";
import styles from "./AuthMenu.module.css";

export function AuthMenu(): React.ReactElement {
  const app = useApp();
  const { auth } = app;

  if (!auth.user) {
    return (
      <div className={styles.root}>
        <button
          type="button"
          className={styles.signIn}
          onClick={() => app.setSignInOpen(true)}
          title="Sign in to administer users and see your desk's RFQs"
        >
          <span aria-hidden>⊙</span>
          Sign in
        </button>
      </div>
    );
  }

  const roleLabel = auth.isAdmin ? "Administrator" : "Trader";
  return (
    <div className={styles.root}>
      <div className={styles.identity}>
        <span className={styles.who}>
          <span className={styles.email}>{auth.user.email}</span>
          <span className={`${styles.role} ${auth.isAdmin ? styles.roleAdmin : ""}`}>
            {roleLabel}
          </span>
        </span>
        <button
          type="button"
          className={styles.signOut}
          onClick={() => void auth.logout()}
          disabled={auth.busy}
          title="Sign out"
        >
          Sign out
        </button>
      </div>
    </div>
  );
}
