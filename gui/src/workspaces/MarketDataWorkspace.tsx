/**
 * MarketDataWorkspace — the ONE class-parametric MARKET-DATA workspace
 * (`fe-fi-migration` #2), under HARD VERTICAL ASSET SEPARATION (W1). There is no
 * FX-vs-FI market-data split and no in-screen cross-asset toggle any more: the
 * workspace renders EXACTLY ONE market-data lens, DERIVED STRICTLY from the active
 * domain — FX (options / metals) → the broker vol-surface (`SurfaceWorkspace`:
 * mark → arb-gate → publish, the 3D `VolSurface3D` + `VolSmile` family, the
 * vol-cube drill-in); fixed-income → the rates curve surface (`CurveWorkspace`:
 * the pillar editor + build-by-instrument-reference + the `YieldCurve` term
 * structure). Under Fixed Income the screen shows ONLY the curve; under FX Options
 * ONLY the vol-surface. The derived lens is gated by the entitlement×license the
 * rail uses, so a rates desk marks its curve under a fixed-income license and an
 * FX desk never sees a lens it cannot use; when the derived lens's class is not
 * viewable, the honest empty-state is shown rather than a faked view.
 *
 * The lens follows `app.activeDomain` (already deep-link-seeded), so first paint
 * matches the active domain with no toggle to fight. `initialLens` (stories/tests)
 * may override the domain-derived lens for a fixed render, but never reintroduces
 * a user-facing switch.
 *
 * The two lenses are the UNCHANGED `SurfaceWorkspace` and `CurveWorkspace`
 * components composed as lens bodies — all of the vol-surface marking / publish /
 * arb-banner / 3D / smile / cube capability AND the full curve pillar-editor /
 * instrument-reference mode / YieldCurve are preserved verbatim; nothing is
 * re-implemented here (the shell is purely the class-parametric gate).
 */

import { useMemo } from "react";
import { useApp } from "../app/AppContext";
import type { CapabilityAsset } from "../data/contract";
import { configuredLicense, type LicensePredicate } from "../lib/commands";
import { SurfaceWorkspace } from "./SurfaceWorkspace";
import { CurveWorkspace } from "./CurveWorkspace";
import styles from "./MarketDataWorkspace.module.css";

/** The asset-class lens the shared Market Data workspace renders under. */
export type MarketDataLens = "fx" | "rates";

export function MarketDataWorkspace({
  initialLens,
}: {
  initialLens?: MarketDataLens;
} = {}): React.ReactElement {
  const app = useApp();
  const licensed: LicensePredicate = useMemo(() => configuredLicense(), []);

  // Hard vertical asset separation (W1): the lens is DERIVED STRICTLY from the
  // active domain — fixed_income → the rates (curve) lens, otherwise → the FX
  // (vol-surface) lens. There is NO in-screen cross-asset toggle; each domain
  // renders only its own asset. An explicit `initialLens` (stories/tests) may
  // override the domain-derived lens for a fixed render, but never reintroduces a
  // user-facing switch.
  const lens: MarketDataLens = initialLens ?? (app.activeDomain === "fixed_income" ? "rates" : "fx");
  const asset: CapabilityAsset = lens === "rates" ? "fixed_income" : "fx_options";

  // Honest gate for the DERIVED lens only: render it iff the identity can VIEW its
  // asset class AND the firm is licensed for it (entitlement-first, license-second
  // — the rail's gate). Otherwise show the honest empty-state, never a faked view.
  if (!(app.auth.can("view", asset) && licensed(asset))) {
    return <div className={styles.loading}>No market-data lens available for your entitlements.</div>;
  }

  return (
    <div className={styles.classShell}>
      <div className={styles.lensBody}>
        {lens === "fx" ? <SurfaceWorkspace /> : <CurveWorkspace />}
      </div>
    </div>
  );
}
