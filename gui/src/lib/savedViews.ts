/**
 * savedViews.ts — the (scope × view-arrangement × analytics) ⇄ URL + localStorage
 * codec (GW1). Any triple of {active workspace, scope drill path + group-by,
 * inspector analytics selection} is a NAMED, bookmarkable, reproducible state:
 * the URL search-params are the CANONICAL form (paste a link, get the exact view);
 * localStorage mirrors the user's saved set so named views survive a reload.
 *
 * This exceeds bare "360T reproducibility" (a view recalled by id): the full state
 * round-trips through the URL with NO server lookup, so a link IS the view.
 *
 * VERIFICATION (GW-FOUNDATION-PLAN §2, saved-views row): the oracle is the
 * ROUND-TRIP IDENTITY `decode(encode(s)) deep-equals s` over a generated state
 * space PLUS ≥3 hand-pinned literal URL⇄state fixtures (external truth, like a
 * golden vector) so a SYMMETRIC encoder/decoder bug is caught by the frozen
 * string. Forward-compat: unknown params are ignored, missing params default —
 * `decode` never throws.
 */

import {
  decodeScopePath,
  encodeScopePath,
  isScopeGroupBy,
  type ScopeGroupBy,
  type ScopeNode,
  type ScopeState,
} from "./scope";
import { DOMAINS, type Domain } from "./commands";

/**
 * The rail workspace the view is parked on. Mirrors `commands.WorkspaceId` exactly
 * (fe-fi-migration #6: the single class-parametric rail). A recalled/pasted URL
 * carrying a retired id (`rates`/`curve`/`ratesrisk`/`deals`/`ratesbook`) is an
 * unknown token ⇒ `decode` ignores it and defaults the workspace (forward-compat),
 * so old bookmarks still open — just on the shared workspace's default lens.
 */
export type WorkspaceId =
  | "ticket"
  | "stream"
  | "surface"
  | "risk"
  | "book"
  | "quoting"
  | "fistreaming"
  | "aggbook"
  | "tiering"
  | "riskbooks"
  | "riskdashboard"
  | "filedgers"
  | "riskrouting"
  | "hedging"
  | "acceptance"
  | "risktransfer"
  | "transferinbox"
  | "transferaudit"
  | "xva"
  | "excel"
  | "clientflow"
  | "latencyops"
  | "streetliquidity"
  | "eventtrace"
  | "connections"
  | "admin"
  | "permissions"
  | "pricinggroups"
  | "corpactions"
  | "refdata";

const WORKSPACES: readonly WorkspaceId[] = [
  "ticket",
  "stream",
  "surface",
  "risk",
  "book",
  "quoting",
  "fistreaming",
  "aggbook",
  "tiering",
  "riskbooks",
  "riskdashboard",
  "filedgers",
  "riskrouting",
  "hedging",
  "acceptance",
  "risktransfer",
  "transferinbox",
  "transferaudit",
  "xva",
  "excel",
  "clientflow",
  "latencyops",
  "streetliquidity",
  "eventtrace",
  "connections",
  "admin",
  "permissions",
  "pricinggroups",
  "corpactions",
  "refdata",
] as const;

/**
 * The analytics selection an inspector strip captures — the lane-agnostic axes a
 * saved view restores. Every field is optional: a lane fills only what applies
 * (a cube has axes; a ticket does not), and an absent field decodes to undefined
 * (the lane's own default). Kept as a flat string map so the codec is forward-
 * compatible — a new analytics axis is a new key, old links still decode.
 */
export interface AnalyticsSelection {
  /** The smile/calibration model id (Surface/Cube), e.g. "MARKET_HEDGE". */
  model?: string;
  /** The selected measure set id (Risk/Book), lane-defined. */
  measures?: string;
  /** The pivot axes id (Cube), lane-defined. */
  axes?: string;
  /** The trend window id (Stream/Surface), e.g. "1m". */
  trend?: string;
}

/** The full reproducible view state: where + what-slice + what-analytics. */
export interface ViewState {
  workspace: WorkspaceId;
  /**
   * The active product-DOMAIN tab (fe-fi-migration re-add). Restored alongside the
   * workspace so a deep-link/saved-view recalls the correct tab AND, for a shared
   * screen, its FX/FI lens (Model A). Defaults to `fx_options` when absent (an old
   * link predating the tab bar decodes to the FX tab — forward-compatible).
   */
  domain: Domain;
  scope: ScopeState;
  analytics: AnalyticsSelection;
}

/** A named, persisted view: an id/label plus its captured state. */
export interface SavedView {
  id: string;
  name: string;
  state: ViewState;
}

// --- URL param keys (the canonical wire form) --------------------------------

// NOTE: the workspace key is `view`, NOT `ws` — `ws` is already the transport's
// live-WebSocket-URL param (data/transportConfig.ts), so a saved-view link must
// not clobber it. The saved-view params are orthogonal to the transport params.
const PARAM = {
  workspace: "view",
  domain: "dom",
  scope: "scope",
  groupBy: "group",
  model: "model",
  measures: "meas",
  axes: "axes",
  trend: "trend",
} as const;

/** The default domain tab — an absent `dom` param (and old links) decode to this. */
const DEFAULT_DOMAIN: Domain = "fx_options";

/** The valid domain ids (from the single {@link DOMAINS} source), for decode guarding. */
const DOMAIN_IDS: ReadonlySet<string> = new Set(DOMAINS.map((d) => d.id));

function isDomain(s: string | null): s is Domain {
  return s !== null && DOMAIN_IDS.has(s);
}

const ANALYTICS_PARAMS: readonly (keyof AnalyticsSelection)[] = [
  "model",
  "measures",
  "axes",
  "trend",
];

function isWorkspace(s: string | null): s is WorkspaceId {
  return s !== null && (WORKSPACES as readonly string[]).includes(s);
}

/**
 * Encode a view to URLSearchParams (the canonical form). Only present fields are
 * written (a clean, minimal URL): the firm-root scope writes no `scope` token,
 * `groupBy:"none"` writes nothing, and absent analytics axes write nothing. The
 * params are emitted in a STABLE key order so two equal states encode to the
 * byte-identical query string (the frozen-fixture contract).
 */
export function encodeView(state: ViewState): URLSearchParams {
  const p = new URLSearchParams();
  p.set(PARAM.workspace, state.workspace);
  // Emit `dom` only for a NON-default domain, so an FX-default view's URL stays
  // byte-identical to the pre-tab-bar form (the default decodes back to fx_options).
  if (state.domain !== DEFAULT_DOMAIN) p.set(PARAM.domain, state.domain);
  const scopeTok = encodeScopePath(state.scope.path);
  if (scopeTok.length > 0) p.set(PARAM.scope, scopeTok);
  if (state.scope.groupBy !== "none") p.set(PARAM.groupBy, state.scope.groupBy);
  const a = state.analytics;
  if (a.model !== undefined) p.set(PARAM.model, a.model);
  if (a.measures !== undefined) p.set(PARAM.measures, a.measures);
  if (a.axes !== undefined) p.set(PARAM.axes, a.axes);
  if (a.trend !== undefined) p.set(PARAM.trend, a.trend);
  return p;
}

/** The canonical query STRING for a view (sorted, stable; no leading `?`). */
export function encodeViewString(state: ViewState): string {
  return encodeView(state).toString();
}

/**
 * Decode a view from URLSearchParams (or a query string). FORWARD-COMPATIBLE:
 * unknown params are ignored, a missing/invalid workspace defaults to "stream"
 * (the app's first-load view), a missing scope defaults to the firm root, and an
 * invalid group-by relaxes to "none". Never throws.
 */
export function decodeView(input: URLSearchParams | string): ViewState {
  const p = typeof input === "string" ? new URLSearchParams(input) : input;

  const wsRaw = p.get(PARAM.workspace);
  const workspace: WorkspaceId = isWorkspace(wsRaw) ? wsRaw : "stream";

  const domRaw = p.get(PARAM.domain);
  const domain: Domain = isDomain(domRaw) ? domRaw : DEFAULT_DOMAIN;

  const path: ScopeNode[] = decodeScopePath(p.get(PARAM.scope) ?? "");
  const groupRaw = p.get(PARAM.groupBy);
  const groupBy: ScopeGroupBy = groupRaw !== null && isScopeGroupBy(groupRaw) ? groupRaw : "none";

  const analytics: AnalyticsSelection = {};
  const model = p.get(PARAM.model);
  if (model !== null) analytics.model = model;
  const measures = p.get(PARAM.measures);
  if (measures !== null) analytics.measures = measures;
  const axes = p.get(PARAM.axes);
  if (axes !== null) analytics.axes = axes;
  const trend = p.get(PARAM.trend);
  if (trend !== null) analytics.trend = trend;

  return { workspace, domain, scope: { path, groupBy }, analytics };
}

/** Drop the analytics axes that have no value (normalises for deep-equality). */
export function normalizeAnalytics(a: AnalyticsSelection): AnalyticsSelection {
  const out: AnalyticsSelection = {};
  for (const k of ANALYTICS_PARAMS) {
    const v = a[k];
    if (v !== undefined) out[k] = v;
  }
  return out;
}

// --- localStorage persistence of the NAMED set -------------------------------

const STORE_KEY = "celnet.savedViews";

/**
 * Load the persisted named views. Tolerant: a missing/corrupt store yields `[]`
 * (never throws), and any entry that doesn't structurally validate is dropped —
 * an old link format can't crash the loader (forward-compat at the storage layer).
 */
export function loadSavedViews(): SavedView[] {
  try {
    const raw = localStorage.getItem(STORE_KEY);
    if (raw === null) return [];
    const parsed: unknown = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [];
    const out: SavedView[] = [];
    for (const entry of parsed) {
      const v = coerceSavedView(entry);
      if (v) out.push(v);
    }
    return out;
  } catch {
    return [];
  }
}

/**
 * Persist the named view set (best-effort; storage may be unavailable). Each
 * entry is stored as `{id, name, q}` where `q` is the CANONICAL query string (the
 * same form the URL uses), so the localStorage mirror and the URL share ONE codec
 * — a view recalled from storage decodes identically to one recalled from a link.
 */
export function storeSavedViews(views: SavedView[]): void {
  try {
    localStorage.setItem(STORE_KEY, JSON.stringify(serializeSavedViews(views)));
  } catch {
    /* storage may be unavailable; the in-session set still holds. */
  }
}

/** The stored shape: id/name + the canonical query string for the view state. */
interface StoredView {
  id: string;
  name: string;
  q: string;
}

function coerceSavedView(entry: unknown): SavedView | null {
  if (typeof entry !== "object" || entry === null) return null;
  const e = entry as Partial<StoredView>;
  if (typeof e.id !== "string" || typeof e.name !== "string" || typeof e.q !== "string") {
    return null;
  }
  return { id: e.id, name: e.name, state: decodeView(e.q) };
}

/** Serialise the named set for storage (state → canonical query string). */
export function serializeSavedViews(views: SavedView[]): StoredView[] {
  return views.map((v) => ({ id: v.id, name: v.name, q: encodeViewString(v.state) }));
}
