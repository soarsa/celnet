/**
 * App root — wires the appearance system and the app context provider around the
 * single-window Shell. The entire trader workspace lives under one provider so
 * the rail, command palette, and pair switcher re-target one shared state.
 *
 * It also owns the connection lifecycle: the transport is resolved ONCE here and
 * shared with both the provider tree and the `useConnectionStatus` monitor. When
 * the live backend drops, a blocking `ReconnectOverlay` covers the (now stale)
 * workspace and counts down the reconnect window; if the window elapses without
 * recovery the app falls back to the `LoginScreen`, from which signing in re-opens
 * a clean session.
 */

import { useEffect, useState } from "react";
import { AppProvider, useApp } from "./app/AppContext";
import { AcceptanceSeedProvider } from "./app/AcceptanceSeedContext";
import { HedgeSeedProvider } from "./app/HedgeSeedContext";
import { LoginScreen } from "./app/LoginScreen";
import { LoginView } from "./app/LoginView";
import { ReconnectOverlay } from "./app/ReconnectOverlay";
import { Shell } from "./app/Shell";
import { UpdateBanner } from "./app/UpdateBanner";
import { resolveTransport } from "./data/transportConfig";
import { resetAndReloadTo, useVersionWatch } from "./data/versionManifest";
import { useAppearance } from "./design/appearance";
import { useDensity } from "./design/density";
import { useConnectionStatus } from "./hooks/useConnectionStatus";

export function App(): React.ReactElement {
  // Initialize the appearance attributes on first paint (Dark default).
  const { appearance, contrast } = useAppearance();
  // The orthogonal density axis (data-density; comfortable default). Mounting the
  // hook applies the cascade attribute on first paint, just like appearance. This
  // is the ONE allowed JS touch-point for density (the GW0 cascade-disjointness
  // contract): the value + toggle are passed into the provider so components
  // consume `app.density` WITHOUT importing the hook (nothing else reads density
  // in JS — the CSS cascade does the work).
  const { density, toggleDensity } = useDensity();
  useEffect(() => {
    document.documentElement.setAttribute("data-appearance", appearance);
    document.documentElement.setAttribute("data-contrast", contrast === "high" ? "high" : "normal");
  }, [appearance, contrast]);

  // Resolve the transport ONCE at the root so the same socket backs both the
  // provider tree (below) and the connection monitor (here). `useState(initFn)`
  // runs the resolver exactly once.
  const [{ transport }] = useState(resolveTransport);
  const connection = useConnectionStatus(transport);

  // Watch the deploy's /version.json: when a newer build (by buildTime) lands under
  // the running page, the UpdateBanner announces it and auto-reloads onto the fresh
  // bundle after a short grace — a cache-busting reload, guarded to fire at most once
  // per detected build (see resetAndReloadTo), so it can never spin.
  const version = useVersionWatch(transport);
  const pendingRelease = version.available;

  // Once the reconnect window elapses, drop to the sign-in screen. Latched in
  // local state so a late background reconnect does not yank the trader back into
  // a workspace they were already signed out of.
  const [signedOut, setSignedOut] = useState(false);
  useEffect(() => {
    if (connection.phase === "failed") setSignedOut(true);
  }, [connection.phase]);

  if (signedOut) {
    // A full reload re-runs `resolveTransport` and re-dials the edge from a clean
    // state — no stale socket, subscriptions, or in-flight waiters carried over.
    return <LoginScreen onSignIn={() => window.location.reload()} />;
  }

  return (
    <AppProvider transport={transport} density={density} toggleDensity={toggleDensity}>
      <AuthGate>
        <AcceptanceSeedProvider>
          <HedgeSeedProvider>
          <Shell />
          {connection.phase === "reconnecting" && (
          <ReconnectOverlay
            remainingSeconds={connection.remainingSeconds}
            endpointLabel={transport.label}
            onSignInNow={() => setSignedOut(true)}
          />
        )}
          {pendingRelease && (
            <UpdateBanner
              release={pendingRelease}
              onReload={() => void resetAndReloadTo(pendingRelease)}
            />
          )}
          </HedgeSeedProvider>
        </AcceptanceSeedProvider>
      </AuthGate>
    </AppProvider>
  );
}

/**
 * Gate the workspace behind a server-enforced session: until a user signs in
 * (`auth.user` is set), the full-page {@link LoginView} is shown instead of the
 * Shell. Rendered INSIDE the provider so it can read the shared `auth` state.
 */
function AuthGate({ children }: { children: React.ReactNode }): React.ReactElement {
  const { auth } = useApp();
  if (!auth.user) return <LoginView />;
  return <>{children}</>;
}
