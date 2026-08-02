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

import { useCallback, useMemo, useState } from "react";

import type {
  Capability,
  CapabilityAction,
  CapabilityAsset,
  UserDesc,
} from "../data/contract";
import type { CelnetTransport } from "../data/transport";
import { can as capabilitySetHas } from "../lib/capabilityMatrix";

/** Narrow an unknown thrown value to a display string. */
function messageOf(error: unknown): string {
  return error instanceof Error ? error.message : "sign-in failed";
}

/** The session-identity API the Shell + Admin workspace consume. */
export interface AuthApi {
  /** The signed-in user, or `null` when anonymous. */
  user: UserDesc | null;
  /**
   * Whether there is a signed-in identity (`user !== null`). Distinct from
   * {@link can}, which is permissive when anonymous: `signedIn` lets navigation
   * gating keep the delegable ADMIN-domain surfaces deny-by-default pre-login even
   * though `can` returns `true` for everything when signed out (see `NavAuth`).
   */
  signedIn: boolean;
  /** Whether the signed-in user is an administrator (false when anonymous). */
  isAdmin: boolean;
  /**
   * The signed-in user's OWN effective capability set (`LoginResult.capabilities`
   * — the server-resolved `role bundle ∪ grants ∖ denies`). Empty when anonymous.
   * The single source the client gates affordances on; the server still enforces.
   */
  capabilities: Capability[];
  /**
   * Whether the signed-in user holds `action` on `asset`. Anonymous ⇒ the legacy
   * permissive path runs un-gated (the server admits the anonymous/demo edge), so
   * `can` returns `true` when signed out — gating only ever NARROWS a real
   * identity's affordances, never the pre-sign-in workspace.
   */
  can: (action: CapabilityAction, asset: CapabilityAsset) => boolean;
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
  const [capabilities, setCapabilities] = useState<Capability[]>([]);
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
        // Retain the caller's OWN effective capability set for affordance gating.
        setCapabilities(result.capabilities);
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
      setCapabilities([]);
      setError(null);
      setBusy(false);
    }
  }, [transport]);

  const clearError = useCallback(() => setError(null), []);

  // Anonymous (signed-out) sessions run the server's permissive/legacy path, so
  // `can` is permissive when there is no identity — gating only ever NARROWS a
  // real signed-in user, never the optional pre-sign-in workspace.
  const can = useMemo(
    () =>
      (action: CapabilityAction, asset: CapabilityAsset): boolean =>
        user === null ? true : capabilitySetHas(capabilities, action, asset),
    [user, capabilities],
  );

  return {
    user,
    signedIn: user !== null,
    isAdmin: user?.role === "ADMIN",
    capabilities,
    can,
    busy,
    error,
    login,
    logout,
    clearError,
  };
}
