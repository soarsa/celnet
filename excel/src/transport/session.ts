/**
 * UserSession — the single owner of the signed-in identity for the add-in,
 * shared (via the runtime singleton in `functions/runtime.ts`) by BOTH the task
 * pane AND the CELNET.* worksheet functions, since the add-in runs them in one
 * Office shared runtime.
 *
 * Sign-in exchanges email + password for a server-minted bearer token
 * (`AuthService.Login`); on success the session installs the token on the shared
 * transport (`Connection.setSessionToken`) so every subsequent gated RPC — from
 * the pane AND from cells — authenticates server-side, and retains the caller's
 * OWN effective capability set for affordance gating. Sign-out invalidates the
 * token server-side (best-effort) and clears it locally.
 *
 * The token is held ONLY in the transport + this in-memory session — never
 * persisted to workbook/Office storage — so a reload re-authenticates from a
 * clean state. Anonymous (signed-out) sessions run the server's permissive price-
 * preview path exactly as before; gating only ever NARROWS a real signed-in
 * identity (GUI parity, `gui/src/hooks/useAuth.ts`), except that the task pane's
 * DEALING affordances additionally require a signed-in identity (a coherent
 * "sign in before you deal" posture — see {@link ENTRY_POINTS} `requiresSignIn`).
 *
 * This is UX only: the server still enforces every RPC.
 */

import type { Connection } from "./connection";
import {
  ENTRY_POINTS,
  can as capabilitySetHas,
  capabilityDenialTitle,
  entrySignInPrompt,
  type Capability,
  type CapabilityAction,
  type CapabilityAsset,
  type EntryPointId,
  type LoginResult,
  type UserDesc,
} from "../contract/access";

/** A monotonic wall-clock in epoch NANOSECONDS (injectable so expiry is testable). */
export type NanoClock = () => bigint;

/** The default nanos clock derived from `Date.now()` (ms → ns). */
const defaultNanoClock: NanoClock = () => BigInt(Date.now()) * 1_000_000n;

/** A snapshot of the live session, delivered to subscribers on every change. */
export interface SessionSnapshot {
  /** The signed-in user, or `null` when anonymous. */
  readonly user: UserDesc | null;
  /** The signed-in user's effective capability set (empty when anonymous). */
  readonly capabilities: readonly Capability[];
  /** Whether a signed-in session has passed its absolute expiry. */
  readonly expired: boolean;
}

export class UserSession {
  private user: UserDesc | null = null;
  private capabilities: readonly Capability[] = [];
  private expiresNanos = 0n;
  private readonly conn: Connection;
  private readonly clock: NanoClock;
  private readonly listeners = new Set<(snap: SessionSnapshot) => void>();

  constructor(conn: Connection, opts?: { readonly clock?: NanoClock }) {
    this.conn = conn;
    this.clock = opts?.clock ?? defaultNanoClock;
  }

  /** Whether an identity is currently signed in (regardless of expiry). */
  isSignedIn(): boolean {
    return this.user !== null;
  }

  /** Whether the signed-in session has passed its absolute expiry. */
  isExpired(): boolean {
    return this.user !== null && this.clock() >= this.expiresNanos;
  }

  /** The signed-in user, or `null` when anonymous. */
  currentUser(): UserDesc | null {
    return this.user;
  }

  /** The signed-in user's effective capability set (empty when anonymous). */
  effectiveCapabilities(): readonly Capability[] {
    return this.capabilities;
  }

  /** A snapshot of the live session. */
  snapshot(): SessionSnapshot {
    return {
      user: this.user,
      capabilities: this.capabilities,
      expired: this.isExpired(),
    };
  }

  /**
   * Record a successful login: retain the identity + effective capabilities and
   * install the bearer token on the shared transport (a signed-in user's token
   * takes precedence over any pre-minted `ConnectionOptions` token). The new token
   * takes effect on the next (re)dial — the contract pins the caller at session
   * open, exactly like the SDK/GUI.
   */
  signIn(result: LoginResult): void {
    this.user = result.user;
    this.capabilities = result.capabilities;
    this.expiresNanos = result.expiresNanos;
    this.conn.setSessionToken(result.token);
    this.notify();
  }

  /**
   * Clear the local identity + token. The caller performs the best-effort
   * server-side invalidation (`Connection.logout`) BEFORE calling this, so even if
   * that round-trip fails the client is unambiguously signed out locally.
   */
  signOut(): void {
    this.user = null;
    this.capabilities = [];
    this.expiresNanos = 0n;
    this.conn.setSessionToken(null);
    this.notify();
  }

  /**
   * Whether the current identity holds `action` on `asset`. Anonymous ⇒ the
   * permissive price-preview path runs un-gated (the server admits the
   * anonymous/demo edge), so this returns `true` when signed out — gating only
   * NARROWS a real identity (GUI parity). A signed-in but EXPIRED session denies
   * everything (forcing a re-login); a signed-in live session tests membership in
   * the effective set.
   */
  can(action: CapabilityAction, asset: CapabilityAsset): boolean {
    if (this.user === null) return true; // anonymous: permissive (server enforces)
    if (this.isExpired()) return false; // expired: deny all → re-login
    return capabilitySetHas(this.capabilities, action, asset);
  }

  /**
   * Whether a concrete affordance is permitted. Dealing affordances
   * (`requiresSignIn`) additionally require a live signed-in identity — anonymous
   * ⇒ `false` (must sign in first). Cell affordances inherit the permissive
   * anonymous posture via {@link can}.
   */
  canEntry(id: EntryPointId): boolean {
    const e = ENTRY_POINTS[id];
    if (e.requiresSignIn && (this.user === null || this.isExpired())) return false;
    return this.can(e.action, e.asset);
  }

  /**
   * The explanation to show on a DENIED affordance — never a silent no-op:
   *   * a signed-in session past expiry ⇒ a re-login prompt;
   *   * a dealing affordance while anonymous ⇒ a "sign in to …" prompt;
   *   * a held identity lacking the capability ⇒ the capability denial sentence.
   * Returns `""` when the affordance is permitted (no tooltip needed).
   */
  entryDenialReason(id: EntryPointId): string {
    if (this.canEntry(id)) return "";
    const e = ENTRY_POINTS[id];
    if (this.user !== null && this.isExpired()) {
      return "Your session has expired — sign in again to continue.";
    }
    if (e.requiresSignIn && this.user === null) {
      return entrySignInPrompt(id);
    }
    return capabilityDenialTitle(e.action, e.asset);
  }

  /** Subscribe to session changes; returns an unsubscribe. */
  subscribe(listener: (snap: SessionSnapshot) => void): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  private notify(): void {
    const snap = this.snapshot();
    for (const l of this.listeners) l(snap);
  }
}
