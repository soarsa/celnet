/**
 * PricingControlProvider — the client-side slice for the firm-wide pricing
 * kill-switch (server `pricing_control` push + `set_pricing_control` mutation).
 *
 * It owns the single live `{ outboundEnabled, inboundEnabled, version }` state,
 * hydrated from the unsolicited `pricing_control` broadcast (the connect-time
 * frame is the initial state — no subscribe verb) and RECONCILED by monotonic
 * `version`: a frame whose version is not newer than the last seen is ignored, so
 * a stale / out-of-order push never regresses the displayed halt state.
 *
 * Both the global {@link PricingHaltBanner} (shown to EVERY user) and the gated
 * toolbar {@link PricingControlMenu} (operable only with
 * `manage_liquidity·fixed_income`) read this one slice, so the control always
 * reflects the live server truth — never merely optimistic local state.
 */

import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState } from "react";
import { useApp } from "./AppContext";
import type { PricingControl } from "../data/contract";

/** The initial (pre-push) state: everything enabled at version 0 so the first
 * server push (version ≥ 1) always wins the reconciliation. */
const INITIAL_CONTROL: PricingControl = {
  outboundEnabled: true,
  inboundEnabled: true,
  version: 0,
};

/**
 * Reconcile an inbound pushed / committed state against the current one by the
 * monotonic `version`: keep `next` only when it is strictly newer. Pure + exported
 * so the version-gating is unit-testable without a render.
 */
export function reconcilePricingControl(
  prev: PricingControl,
  next: PricingControl,
): PricingControl {
  return next.version > prev.version ? next : prev;
}

/** The halt classification derived from a control state (pure, exported for tests). */
export type HaltLevel = "none" | "outbound" | "inbound" | "all";

export function haltLevel(control: PricingControl): HaltLevel {
  if (!control.outboundEnabled && !control.inboundEnabled) return "all";
  if (!control.outboundEnabled) return "outbound";
  if (!control.inboundEnabled) return "inbound";
  return "none";
}

/** The context surface both the banner and the toolbar control consume. */
export interface PricingControlContextValue {
  /** The live reconciled kill-switch state. */
  readonly control: PricingControl;
  /** The derived halt classification (`none` ⇒ nothing halted). */
  readonly level: HaltLevel;
  /** Whether ANY gate is halted (outbound and/or inbound). */
  readonly halted: boolean;
  /** True while a `setPricingControl` round-trip is in flight. */
  readonly pending: boolean;
  /** The last mutation error message, or `null`. */
  readonly error: string | null;
  /**
   * Mutate the firm-wide state. Resolves once the server commits (the committed
   * state is reconciled in immediately); rejects are captured into {@link error}.
   */
  setPricingControl: (outboundEnabled: boolean, inboundEnabled: boolean) => Promise<void>;
}

const PricingControlContext = createContext<PricingControlContextValue | null>(null);

export function PricingControlProvider({
  children,
}: {
  children: React.ReactNode;
}): React.ReactElement {
  const app = useApp();
  const [control, setControl] = useState<PricingControl>(INITIAL_CONTROL);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Subscribe to the unsolicited push for the life of the transport. Not gated on
  // sign-in: the halt banner reflects the firm-wide state for everyone. The
  // reconcile is version-gated so re-delivery / reconnect replays never regress it.
  useEffect(() => {
    const dispose = app.transport.subscribePricingControl((next) => {
      setControl((prev) => reconcilePricingControl(prev, next));
    });
    return dispose;
  }, [app.transport]);

  // Keep the latest control in a ref so the setter never needs to be re-created
  // when the state changes (stable identity for memoized consumers).
  const controlRef = useRef(control);
  controlRef.current = control;

  const setPricingControl = useCallback(
    async (outboundEnabled: boolean, inboundEnabled: boolean): Promise<void> => {
      setPending(true);
      setError(null);
      try {
        const committed = await app.transport.setPricingControl(outboundEnabled, inboundEnabled);
        // Fold the committed reply in immediately (the broadcast push arrives too,
        // and the version gate makes the double-apply idempotent).
        setControl((prev) => reconcilePricingControl(prev, committed));
      } catch (err) {
        setError(err instanceof Error ? err.message : "could not update pricing control");
        throw err;
      } finally {
        setPending(false);
      }
    },
    [app.transport],
  );

  const value = useMemo<PricingControlContextValue>(() => {
    const level = haltLevel(control);
    return {
      control,
      level,
      halted: level !== "none",
      pending,
      error,
      setPricingControl,
    };
  }, [control, pending, error, setPricingControl]);

  return (
    <PricingControlContext.Provider value={value}>{children}</PricingControlContext.Provider>
  );
}

/** Read the pricing-control slice. Throws if used outside the provider. */
export function usePricingControl(): PricingControlContextValue {
  const ctx = useContext(PricingControlContext);
  if (ctx === null) {
    throw new Error("usePricingControl must be used within a PricingControlProvider");
  }
  return ctx;
}
