/**
 * Pure logic for the MOBILE read-only status board (`app/mobile/MobileStatusApp`).
 *
 * The board is a thin presentational layer over the SAME read-only stores the
 * desktop Risk surface consumes — per-portfolio risk (`listRiskBookRisk`), client
 * deals (`listDeals`), and the fired-hedge audit trail (`listHedgeProvenance`). This
 * module holds every pure decision the board makes so it can be unit-tested without a
 * DOM or a transport: which asset tabs the identity may see, how the firm-wide
 * risk/deal/hedge rows split by asset class, the per-book RAG folds, and the glance
 * summary strip.
 *
 * Asset reality (see `data/assetClass.ts`): the risk-routing / desk-quoting / rates
 * streams are structurally fixed-income (OIS-only) on this one contract, so every
 * `Deal` classifies `fixed_income` via {@link dealAsset}, and the FX-Options lenses
 * are honestly EMPTY. The board still exposes an FX-Options tab when the identity is
 * entitled to it — it simply shows honest empty states there, exactly as the desktop
 * Book workspace's FX Deals/Positions lenses do.
 */

import type {
  CapabilityAction,
  CapabilityAsset,
  Deal,
  HedgeProvenance,
  RagBand,
  RiskBookRisk,
  RiskLimitUtilization,
} from "../data/contract";
import { dealAsset } from "../data/assetClass";

/** A can-predicate shaped like {@link ../hooks/useAuth.AuthApi.can} (kept structural so this module has no hook dependency). */
export type CanPredicate = (action: CapabilityAction, asset: CapabilityAsset) => boolean;

/** One selectable asset tab: the capability asset id plus its display labels. */
export interface MobileAssetTab {
  id: CapabilityAsset;
  /** The full tab label. */
  label: string;
  /** A compact chip label for tight widths. */
  short: string;
}

/**
 * The asset tabs in display order (Fixed Income first — it is the asset with live
 * risk-routing/deal/hedge data on this contract; FX Options second).
 */
export const MOBILE_ASSET_TABS: readonly MobileAssetTab[] = [
  { id: "fixed_income", label: "Fixed Income", short: "FI" },
  { id: "fx_options", label: "FX Options", short: "FXO" },
];

/** The three read-only sub-views within an asset. */
export type MobileSubView = "risk" | "hedge" | "client";

/** Sub-view display metadata (label + short chip), in display order. */
export const MOBILE_SUB_VIEWS: readonly { id: MobileSubView; label: string; short: string }[] = [
  { id: "risk", label: "Risk", short: "Risk" },
  { id: "hedge", label: "Hedges", short: "Hedges" },
  { id: "client", label: "Client flow", short: "Client" },
];

/**
 * The assets this identity may VIEW, in display order. `can("view", asset)` is the
 * same gate the desktop rail uses (and is permissive when anonymous, so a signed-out
 * session sees both tabs). Desk scoping is enforced server-side on the row data.
 */
export function entitledAssets(can: CanPredicate): CapabilityAsset[] {
  return MOBILE_ASSET_TABS.map((t) => t.id).filter((id) => can("view", id));
}

/**
 * Map each risk portfolio id to the set of asset classes whose routed deals landed in
 * it (`Deal.riskBookId` → {@link dealAsset}). Portfolios with no routed deals are
 * absent — {@link riskRowsForAsset} then treats them as fixed-income (the routing
 * world is FI), never FX.
 */
export function bookAssetIndex(deals: readonly Deal[]): Map<string, Set<CapabilityAsset>> {
  const index = new Map<string, Set<CapabilityAsset>>();
  for (const deal of deals) {
    const bookId = deal.riskBookId;
    if (bookId === undefined || bookId.length === 0) continue;
    const existing = index.get(bookId) ?? new Set<CapabilityAsset>();
    existing.add(dealAsset(deal));
    index.set(bookId, existing);
  }
  return index;
}

/**
 * The risk portfolios to show under `asset`: a book with asset-tagged deals shows
 * under exactly those assets; an UNCLASSIFIED book (no routed deals yet) shows under
 * `fixed_income` only — the risk-routing engine is fixed-income, so an empty book
 * belongs to FI and never fabricates an FX exposure.
 */
export function riskRowsForAsset(
  rows: readonly RiskBookRisk[],
  index: ReadonlyMap<string, Set<CapabilityAsset>>,
  asset: CapabilityAsset,
): RiskBookRisk[] {
  return rows.filter((row) => {
    const tagged = index.get(row.bookId);
    if (tagged && tagged.size > 0) return tagged.has(asset);
    return asset === "fixed_income";
  });
}

/** Client deals for `asset`, classified by dealt instrument family ({@link dealAsset}). */
export function dealsForAsset(deals: readonly Deal[], asset: CapabilityAsset): Deal[] {
  return deals.filter((deal) => dealAsset(deal) === asset);
}

/**
 * Fired hedges for `asset`. The hedge engine is the fixed-income rates
 * warehouse/hedge loop, so its audit trail is fixed-income; the FX-Options lens is
 * honestly empty (there is no FX hedge engine on this contract).
 */
export function hedgesForAsset(
  hedges: readonly HedgeProvenance[],
  asset: CapabilityAsset,
): HedgeProvenance[] {
  return asset === "fixed_income" ? [...hedges] : [];
}

/** RAG band from a utilization fraction (mirrors the server `RagBand::from_fraction`). */
export function ragBandOfFraction(fraction: number): RagBand {
  if (fraction >= 1) return "red";
  if (fraction >= 0.8) return "amber";
  return "green";
}

/** Numeric severity of a band for worst-first comparison (red highest). */
export function bandSeverity(band: RagBand): number {
  return band === "red" ? 2 : band === "amber" ? 1 : 0;
}

/** The worst (highest-severity) band across a portfolio's caps; green when none. */
export function worstBandOf(limits: readonly RiskLimitUtilization[]): RagBand {
  let worst: RagBand = "green";
  for (const limit of limits) {
    if (limit.band === "red") return "red";
    if (limit.band === "amber") worst = "amber";
  }
  return worst;
}

/**
 * The worst (highest) FINITE utilization fraction across a portfolio's caps, in
 * [0, ∞). A breached cap reports `+Infinity` for its fraction (0 cap, used > 0); we
 * clamp that to the count of breaches via {@link worstBandOf} instead, so this stays
 * finite for the "worst utilisation %" summary. Returns 0 when there are no caps.
 */
export function worstFractionOf(limits: readonly RiskLimitUtilization[]): number {
  let worst = 0;
  for (const limit of limits) {
    if (Number.isFinite(limit.fraction) && limit.fraction > worst) worst = limit.fraction;
  }
  return worst;
}

/** A risk portfolio row decorated with its folded RAG state, for the board cards. */
export interface MobileRiskCard {
  row: RiskBookRisk;
  band: RagBand;
  /** The worst finite utilization fraction (0 when no caps / all unevaluable). */
  worstFraction: number;
  /** Whether any cap is breached (band red). */
  breaching: boolean;
}

/** Decorate + sort risk rows WORST-FIRST (breaching, then highest utilisation, then gross). */
export function riskCards(rows: readonly RiskBookRisk[]): MobileRiskCard[] {
  const cards: MobileRiskCard[] = rows.map((row) => {
    const band = worstBandOf(row.limits);
    return {
      row,
      band,
      worstFraction: worstFractionOf(row.limits),
      breaching: band === "red",
    };
  });
  return cards.sort((a, b) => {
    const bySeverity = bandSeverity(b.band) - bandSeverity(a.band);
    if (bySeverity !== 0) return bySeverity;
    const byFraction = b.worstFraction - a.worstFraction;
    if (byFraction !== 0) return byFraction;
    return b.row.grossNotional - a.row.grossNotional;
  });
}

/** The 2-second glance summary across an asset's risk portfolios. */
export interface MobileRiskSummary {
  /** Number of risk portfolios shown. */
  bookCount: number;
  /** Signed sum of net notional across portfolios. */
  netNotional: number;
  /** Sum of gross notional across portfolios. */
  grossNotional: number;
  /** Summed net DV01, or `null` when no portfolio reports one (never a fabricated 0). */
  netDv01: number | null;
  /** Portfolios whose worst band is red (a breach). */
  breaching: number;
  /** The worst finite utilization fraction across every portfolio's caps. */
  worstFraction: number;
  /** The worst band across every portfolio (accounts for +Infinity breaches). */
  worstBand: RagBand;
}

/** Fold an asset's risk rows into the glance summary strip. */
export function summariseRisk(rows: readonly RiskBookRisk[]): MobileRiskSummary {
  let netNotional = 0;
  let grossNotional = 0;
  let netDv01: number | null = null;
  let breaching = 0;
  let worstFraction = 0;
  let worstBand: RagBand = "green";
  for (const row of rows) {
    netNotional += row.netNotional;
    grossNotional += row.grossNotional;
    if (row.dv01 !== null) netDv01 = (netDv01 ?? 0) + row.dv01;
    const band = worstBandOf(row.limits);
    if (band === "red") breaching += 1;
    if (bandSeverity(band) > bandSeverity(worstBand)) worstBand = band;
    const frac = worstFractionOf(row.limits);
    if (frac > worstFraction) worstFraction = frac;
  }
  return {
    bookCount: rows.length,
    netNotional,
    grossNotional,
    netDv01,
    breaching,
    worstFraction,
    worstBand,
  };
}
