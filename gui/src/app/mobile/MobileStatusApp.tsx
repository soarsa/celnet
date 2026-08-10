/**
 * MobileStatusApp — the touch-friendly, READ-ONLY status board shown in place of the
 * desktop `Shell` when a trader opens the app on a phone (see
 * {@link ../../hooks/useMobileLayout}). It is a glanceable risk-and-flow board: a
 * compact header (identity + live status + escape hatch), asset tabs the identity is
 * entitled to (Fixed Income / FX Options), and three read-only sub-views per asset —
 * the per-portfolio RISK board, the HEDGE blotter, and the CLIENT blotter.
 *
 * It is a thin presentational layer over the SAME read-only stores the desktop Risk
 * surface uses (`listRiskBookRisk` / `listDeals` / `listHedgeProvenance`), fetched
 * ONCE here and split by asset via the pure {@link ../../lib/mobileStatus} folds, so
 * switching asset/sub-tab never refetches. No mutating action is ever exposed.
 *
 * Auth/session is unchanged: it runs under the same provider tree as the Shell, so
 * the same login, logout, heartbeat and reconnect overlay apply. "Use full app" is
 * the escape hatch, owned by the app root and threaded in via {@link onUseFullApp}.
 */

import { useMemo, useState } from "react";

import { useApp } from "../AppContext";
import { CelerMark } from "../../components/CelerMark";
import { useCachedResource } from "../../hooks/useCachedResource";
import { useConnectionStatus } from "../../hooks/useConnectionStatus";
import { useAppearance } from "../../design/appearance";
import type {
  Deal,
  HedgeProvenance,
  RiskBook,
  RiskBookRisk,
} from "../../data/contract";
import {
  bookAssetIndex,
  dealsForAsset,
  entitledAssets,
  hedgesForAsset,
  MOBILE_ASSET_TABS,
  MOBILE_SUB_VIEWS,
  riskRowsForAsset,
  type MobileSubView,
} from "../../lib/mobileStatus";
import { MobileRiskBoard } from "./MobileRiskBoard";
import { MobileHedgeBlotter } from "./MobileHedgeBlotter";
import { MobileClientBlotter } from "./MobileClientBlotter";
import { MobileHelpSheet } from "./MobileHelpSheet";
import styles from "./MobileStatusApp.module.css";

/** localStorage flag: the first-open hint has been dismissed. */
const HINT_SEEN_KEY = "celnet.mobile.hintSeen";

function readHintSeen(): boolean {
  try {
    return window.localStorage.getItem(HINT_SEEN_KEY) === "1";
  } catch {
    return true; // storage disabled ⇒ don't nag every open
  }
}

export interface MobileStatusAppProps {
  /** Escape hatch: force the full desktop Shell (owned + persisted by the app root). */
  onUseFullApp: () => void;
}

export function MobileStatusApp({ onUseFullApp }: MobileStatusAppProps): React.ReactElement {
  const app = useApp();
  const { appearance, toggleAppearance } = useAppearance();
  const connection = useConnectionStatus(app.transport);

  // Entitled asset tabs (permissive when anonymous ⇒ both shown). Desk scoping is
  // enforced server-side on the row data below.
  const assets = useMemo(() => entitledAssets(app.auth.can), [app.auth.can]);
  const [assetIdx, setAssetIdx] = useState(0);
  const asset = assets[Math.min(assetIdx, Math.max(assets.length - 1, 0))] ?? "fixed_income";
  const [subView, setSubView] = useState<MobileSubView>("risk");

  // Help + first-open hint. The hint auto-opens once; dismissing it persists the flag.
  const [helpOpen, setHelpOpen] = useState(false);
  const [hintOpen, setHintOpen] = useState<boolean>(() => !readHintSeen());
  const dismissHint = (): void => {
    setHintOpen(false);
    try {
      window.localStorage.setItem(HINT_SEEN_KEY, "1");
    } catch {
      /* best-effort */
    }
  };

  // The shared read-only stores — fetched ONCE (stale-while-revalidate), split by
  // asset with the pure folds. Never mounted alongside the desktop, so distinct keys
  // are safe.
  const risk = useCachedResource<RiskBookRisk[]>("mobile:riskBookRisk", () =>
    app.transport.listRiskBookRisk(),
  );
  const books = useCachedResource<RiskBook[]>("mobile:riskBooks", () =>
    app.transport.listRiskBooks(),
  );
  const dealsRes = useCachedResource<Deal[]>("mobile:deals", () =>
    app.transport.listDeals({}).then((r) => r.deals),
  );
  const hedgeRes = useCachedResource<HedgeProvenance[]>("mobile:hedgeProvenance", () =>
    app.transport.listHedgeProvenance(),
  );

  const allDeals = dealsRes.data ?? [];
  const index = useMemo(() => bookAssetIndex(allDeals), [allDeals]);
  const riskRows = useMemo(
    () => riskRowsForAsset(risk.data ?? [], index, asset),
    [risk.data, index, asset],
  );
  const assetDeals = useMemo(() => dealsForAsset(allDeals, asset), [allDeals, asset]);
  const assetHedges = useMemo(
    () => hedgesForAsset(hedgeRes.data ?? [], asset),
    [hedgeRes.data, asset],
  );
  const bookNames = useMemo(() => {
    const map = new Map<string, string>();
    for (const b of books.data ?? []) map.set(b.id, b.name);
    return map;
  }, [books.data]);

  const statusTone =
    connection.phase === "reconnecting"
      ? "warn"
      : connection.phase === "failed"
        ? "danger"
        : app.transport.label.startsWith("live")
          ? "live"
          : "replay";
  const statusText =
    connection.phase === "reconnecting"
      ? "Reconnecting…"
      : connection.phase === "failed"
        ? "Disconnected"
        : statusTone === "live"
          ? "Live"
          : "Replay";

  const user = app.auth.user;

  return (
    <div className={styles.app}>
      <header className={styles.header}>
        <div className={styles.brandRow}>
          <CelerMark size={22} className={styles.mark} title="Celnet" />
          <span className={styles.wordmark}>Celnet</span>
          <span
            className={styles.status}
            data-tone={statusTone}
            role="status"
            aria-label={`Connection ${statusText}`}
          >
            <span className={styles.statusDot} aria-hidden />
            {statusText}
          </span>
          <div className={styles.spacer} />
          <button
            type="button"
            className={styles.iconBtn}
            onClick={toggleAppearance}
            aria-label="toggle light or dark appearance"
          >
            {appearance === "dark" ? "☾" : "☀"}
          </button>
          <button
            type="button"
            className={styles.iconBtn}
            onClick={() => setHelpOpen(true)}
            aria-label="open help"
          >
            ?
          </button>
        </div>
        <div className={styles.identityRow}>
          <span className={styles.identity}>
            {user ? (
              <>
                <span className={styles.userName}>{user.displayName || user.email}</span>
                {(user.allDesks || user.deskIds.length > 0) && (
                  <>
                    <span className={styles.sep} aria-hidden>
                      ·
                    </span>
                    <span className={styles.desk}>
                      {user.allDesks ? "All desks" : user.deskIds.join(", ")}
                    </span>
                  </>
                )}
              </>
            ) : (
              <span className={styles.userName}>Guest</span>
            )}
          </span>
          <div className={styles.spacer} />
          <button type="button" className={styles.textBtn} onClick={onUseFullApp}>
            Full app ↗
          </button>
          <button
            type="button"
            className={styles.textBtn}
            onClick={() => void app.auth.logout()}
            disabled={!user || app.auth.busy}
          >
            Sign out
          </button>
        </div>
        {assets.length > 1 && (
          <div className={styles.assetTabs} role="tablist" aria-label="asset class">
            {MOBILE_ASSET_TABS.filter((t) => assets.includes(t.id)).map((t) => {
              const active = t.id === asset;
              return (
                <button
                  key={t.id}
                  type="button"
                  role="tab"
                  aria-selected={active}
                  className={`${styles.assetTab} ${active ? styles.assetTabActive : ""}`}
                  onClick={() => setAssetIdx(assets.indexOf(t.id))}
                >
                  {t.label}
                </button>
              );
            })}
          </div>
        )}
        <div className={styles.subTabs} role="tablist" aria-label="status view">
          {MOBILE_SUB_VIEWS.map((v) => {
            const active = v.id === subView;
            return (
              <button
                key={v.id}
                type="button"
                role="tab"
                aria-selected={active}
                className={`${styles.subTab} ${active ? styles.subTabActive : ""}`}
                onClick={() => setSubView(v.id)}
              >
                {v.label}
              </button>
            );
          })}
        </div>
      </header>

      <main className={styles.content}>
        {subView === "risk" && (
          <MobileRiskBoard
            rows={riskRows}
            isLoading={risk.isLoading}
            error={risk.error}
            asset={asset}
          />
        )}
        {subView === "hedge" && (
          <MobileHedgeBlotter
            hedges={assetHedges}
            bookNames={bookNames}
            isLoading={hedgeRes.isLoading}
            error={hedgeRes.error}
            asset={asset}
          />
        )}
        {subView === "client" && (
          <MobileClientBlotter
            deals={assetDeals}
            bookNames={bookNames}
            isLoading={dealsRes.isLoading}
            error={dealsRes.error}
            asset={asset}
          />
        )}
      </main>

      {helpOpen && <MobileHelpSheet variant="help" onClose={() => setHelpOpen(false)} />}
      {hintOpen && !helpOpen && (
        <MobileHelpSheet variant="hint" onClose={dismissHint} />
      )}
    </div>
  );
}
