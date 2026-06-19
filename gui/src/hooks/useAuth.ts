/**
 * useAuth — the single owner of the signed-in identity for the session.
 *
 * Login exchanges email + password for a server-minted bearer token (the
 * `AuthService.Login` contract); on success this hook installs the token on the
 * transport (`setSessionToken`) so every subsequent gated RPC authenticates
 * server-side — and admin RPCs become callable, and the FIX monitor narrows to
 * the user's desk (the Increment-3 desk scoping). Logout invalidates the token
 * server-side and clears it locally.
 *
 * The token is held ONLY in the transport (in memory) — never in this hook's
 * render state, never persisted to local/session storage — so a reload re-
 * authenticates from a clean state and an XSS cannot lift a long-lived session
 * from storage. The hook keeps just the user profile for rendering.
 *
 * Sign-in is OPTIONAL: the app runs anonymously (the server's permissive/legacy
 * path) until a user signs in, exactly as before. Signing in layers a real
 * identity over that path; it does not gate the workspace.
 */

import { useCallback, useState } from "react";

import type { UserDesc } from "../data/contract";
import type { CelnetTransport } from "../data/transport";

/** Narrow an unknown thrown value to a display string. */
function messageOf(error: unknown): string {
  return error instanceof Error ? error.message : "sign-in failed";
}

/** The session-identity API the Shell + Admin workspace consume. */
export interface AuthApi {
  /** The signed-in user, or `null` when anonymous. */
  user: UserDesc | null;
  /** Whether the signed-in user is an administrator (false when anonymous). */
  isAdmin: boolean;
  /** True while a login/logout round-trip is in flight. */
  busy: boolean;
  /** The last sign-in error as a display string, or `null`. */
  error: string | null;
  /** Sign in; resolves on success, rejects (and sets `error`) on failure. */
  login: (email: string, password: string) => Promise<void>;
  /** Sign out (best-effort server invalidation); always clears the local identity. */
  logout: () => Promise<void>;
  /** Clear the current error (e.g. when reopening the sign-in dialog). */
  clearError: () => void;
}

export function useAuth(transport: CelnetTransport): AuthApi {
  const [user, setUser] = useState<UserDesc | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const login = useCallback(
    async (email: string, password: string): Promise<void> => {
      setBusy(true);
      setError(null);
      try {
        const result = await transport.login(email, password);
        // Install the bearer token so every gated RPC authenticates from here on.
        transport.setSessionToken(result.token);
        setUser(result.user);
      } catch (e: unknown) {
        const msg = messageOf(e);
        setError(msg);
        throw new Error(msg);
      } finally {
        setBusy(false);
      }
    },
    [transport],
  );

  const logout = useCallback(async (): Promise<void> => {
    setBusy(true);
    try {
      // Best-effort server-side invalidation; even if it fails we drop the local
      // identity and token so the client is unambiguously signed out.
      await transport.logout().catch(() => false);
    } finally {
      transport.setSessionToken(null);
      setUser(null);
      setError(null);
      setBusy(false);
    }
  }, [transport]);

  const clearError = useCallback(() => setError(null), []);

  return {
    user,
    isAdmin: user?.role === "ADMIN",
    busy,
    error,
    login,
    logout,
    clearError,
  };
}
