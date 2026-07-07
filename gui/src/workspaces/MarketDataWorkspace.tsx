/**
 * MarketDataWorkspace — the ONE class-parametric MARKET-DATA workspace
 * (`fe-fi-migration` #2). There is no FX-vs-FI market-data split any more: a
 * single workspace renders the market-data LENS for the active asset class — FX
 * (options / metals) → the broker vol-surface (`SurfaceWorkspace`: mark →
 * arb-gate → publish, the 3D `VolSurface3D` + `VolSmile` family, the vol-cube
 * drill-in); fixed-income → the rates curve surface (`CurveWorkspace`: the
 * pillar editor + build-by-instrument-reference + the `YieldCurve` term
 * structure). The lens toggle is offered only for the classes the signed-in
 * identity can `view` AND the firm is licensed for (the entitlement×license gate
 * the rail uses), so a rates desk marks its curve here under a fixed-income
 * license and an FX desk never sees a lens it cannot use.
 *
 * `initialLens` is the entry-point default: the `surface` rail row opens the FX
 * vol-surface lens, the `curve` rail row opens the fixed-income curve lens (both
 * mount THIS component via Shell.tsx entry points). A single available lens
 * renders directly, with no toggle. This mirrors the landed RiskWorkspace fold
 * (cl_1f5e26efffee1360: FI is integrated, not a peer) — market data now flows
 * through the SAME workflow across asset classes, folding the FX/FI silo.
 *
 * The two lenses are the UNCHANGED `SurfaceWorkspace` and `CurveWorkspace`
 * components composed as lens bodies — all of the vol-surface marking / publish /
 * arb-banner / 3D / smile / cube capability AND the full curve pillar-editor /
 * instrument-reference mode / YieldCurve are preserved verbatim; nothing is
 * re-implemented here (the shell is purely the class-parametric gate + toggle).
 */

import { useEffect, useMemo, useRef, useState } from "react";
import { useApp } from "../app/AppContext";
import type { CapabilityAsset } from "../data/contract";
import { configuredLicense, type LicensePredicate } from "../lib/commands";
import { SurfaceWorkspace } from "./SurfaceWorkspace";
import { CurveWorkspace } from "./CurveWorkspace";
import styles from "./MarketDataWorkspace.module.css";

/** The asset-class lens the shared Market Data workspace renders under. */
export type MarketDataLens = "fx" | "rates";

/** One row per class the workspace spans: its lens id, capability asset, label. */
const LENSES: readonly { lens: MarketDataLens; asset: CapabilityAsset; label: string }[] = [
  { lens: "fx", asset: "fx_options", label: "FX Options" },
  { lens: "rates", asset: "fixed_income", label: "Fixed Income" },
];

export function MarketDataWorkspace({
  initialLens,
}: {
  initialLens?: MarketDataLens;
} = {}): React.ReactElement {
  const app = useApp();
  const licensed: LicensePredicate = useMemo(() => configuredLicense(), []);
  // A lens is available iff the identity can VIEW its asset class AND the firm is
  // licensed for it — the same entitlement-first, license-second gate the rail
  // applies (lib/commands). Signed out, `can` is permissive ⇒ both lenses show.
  const available = useMemo(
    () => LENSES.filter((l) => app.auth.can("view", l.asset) && licensed(l.asset)),
    [app.auth, licensed],
  );

  // The ENTRY lens: an explicit `initialLens` prop wins (stories/tests/a targeted
  // rail entry); otherwise it is DERIVED from the active domain tab so the FIRST
  // render already matches the (possibly deep-linked) domain — fixed_income → the
  // rates (curve) lens, fx_options → the FX (vol-surface) lens (Model A). The
  // on-domain-change sync below skips mount, so this seed holds on first paint.
  const [lens, setLens] = useState<MarketDataLens>(() => {
    const entry: MarketDataLens =
      initialLens ?? (app.activeDomain === "fixed_income" ? "rates" : "fx");
    return available.some((l) => l.lens === entry) ? entry : (available[0]?.lens ?? "fx");
  });

  // If entitlement/license narrows at runtime so the active lens is gone, clamp to
  // an available one (never strand on a lens the identity cannot use). The Shell
  // unmounts the pane when its class is fully gated, so this is a defensive
  // re-sync, not the primary gate.
  useEffect(() => {
    if (available.length > 0 && !available.some((l) => l.lens === lens)) {
      setLens(available[0]!.lens);
    }
  }, [available, lens]);

  // Model A: pre-select the lens for the ACTIVE DOMAIN tab — fixed_income → the
  // rates lens, fx_options → the FX lens — so flipping tabs while on this shared
  // screen flips the surface's lens. Fires ONLY on a domain CHANGE (the initial
  // mount is skipped so the useState-seeded / `initialLens` lens is honoured, and
  // manual lens selection between switches is never fought), and only switches to a
  // lens the identity has available; the runtime-clamp effect above still guards it.
  const domainSynced = useRef(false);
  // eslint-disable-next-line react-hooks/exhaustive-deps -- sync on domain change only
  useEffect(() => {
    if (!domainSynced.current) {
      domainSynced.current = true;
      return;
    }
    const want: MarketDataLens = app.activeDomain === "fixed_income" ? "rates" : "fx";
    if (available.some((l) => l.lens === want)) setLens(want);
  }, [app.activeDomain]);

  if (available.length === 0) {
    // Defensive: the Shell hides the pane when the class is gated, so this is only
    // reachable in a degenerate mid-transition — shown honestly, never faked.
    return <div className={styles.loading}>No market-data lens available for your entitlements.</div>;
  }

  return (
    <div className={styles.classShell}>
      {available.length > 1 && (
        <div className={styles.lensBar} role="group" aria-label="market data asset class">
          {available.map((l) => (
            <button
              key={l.lens}
              type="button"
              className={`${styles.lensTab} ${lens === l.lens ? styles.lensTabActive : ""}`}
              aria-pressed={lens === l.lens}
              onClick={() => setLens(l.lens)}
            >
              {l.label}
            </button>
          ))}
        </div>
      )}
      <div className={styles.lensBody}>
        {lens === "fx" ? <SurfaceWorkspace /> : <CurveWorkspace />}
      </div>
    </div>
  );
}
