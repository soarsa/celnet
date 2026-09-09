/**
 * Shell — the single-window, multi-pane workspace (GUI-DESIGN §2): a persistent
 * left rail of workspaces, a title + command bar, the active workspace canvas,
 * and a slim status ribbon carrying stream health + the global clock.
 *
 * GW1: the rail and the keyboard grammar are now DATA-DRIVEN from the single
 * command registry (`lib/commands.ts`): the rail iterates `RAIL`, the `⌘N` jumps
 * are derived (`⌘1..n`, uncapping the old `⌘1-5`), and the global key handler is a
 * registry dispatcher (`resolveChord`) so the honoured grammar IS the registry. The
 * scope/underlier is the ONE `ScopeControl` breadcrumb (its leaf is `ScopeSwitcher`),
 * replacing the four redundant pair affordances. Saved views (scope × view ×
 * analytics) are URL-encoded + localStorage-persisted (`SavedViewsMenu`).
 */

import { useEffect, useMemo, useRef, useState } from "react";
import { useApp } from "./AppContext";
import { CommandPalette } from "../components/CommandPalette";
import { ShortcutsOverlay } from "../components/ShortcutsOverlay";
import { HelpCenter } from "../components/HelpCenter";
import { DefaultRoutePrompt } from "../components/DefaultRoutePrompt";
import { TourProvider } from "./TourProvider";
import { useAppearance } from "../design/appearance";
import { TicketWorkspace } from "../workspaces/TicketWorkspace";
import { MarketDataWorkspace } from "../workspaces/MarketDataWorkspace";
import { QuotingWorkspace } from "../workspaces/QuotingWorkspace";
import { CorporateActionsWorkspace } from "../workspaces/CorporateActionsWorkspace";
import { ReferenceDataWorkspace } from "../workspaces/ReferenceDataWorkspace";
import { UnifiedMarketStudio } from "../workspaces/UnifiedMarketStudio";
import { UnifiedPricingStudio } from "../workspaces/UnifiedPricingStudio";
import { UnifiedDistributionStudio } from "../workspaces/UnifiedDistributionStudio";
import { UnifiedBlotterStudio } from "../workspaces/UnifiedBlotterStudio";
import { UnifiedRiskStudio } from "../workspaces/UnifiedRiskStudio";
import { UnifiedPolicyStudio } from "../workspaces/UnifiedPolicyStudio";
import { StreamWorkspace } from "../workspaces/StreamWorkspace";
import { FiStreamingWorkspace } from "../workspaces/FiStreamingWorkspace";
import { AggregatedBookWorkspace } from "../workspaces/AggregatedBookWorkspace";
import { RiskDashboardWorkspace } from "../workspaces/RiskDashboardWorkspace";
import { HedgeFlowWorkspace } from "../workspaces/HedgeFlowWorkspace";
import { HedgingWorkspace } from "../workspaces/hedging/HedgingWorkspace";
import { RiskTransferWorkspace } from "../workspaces/risktransfer/RiskTransferWorkspace";
import { RiskWorkspace } from "../workspaces/RiskWorkspace";
import { XvaWorkspace } from "../workspaces/XvaWorkspace";
import { BookWorkspace } from "../workspaces/BookWorkspace";
import { ConnectionsWorkspace } from "../workspaces/ConnectionsWorkspace";
import { LiquidityWorkspace } from "../workspaces/LiquidityWorkspace";
import { AdminWorkspace } from "../workspaces/AdminWorkspace";
import { PermissionsWorkspace } from "../workspaces/PermissionsWorkspace";
import { PricingGroupsWorkspace } from "../workspaces/PricingGroupsWorkspace";
import { ClientFlowWorkspace } from "../workspaces/analytics/ClientFlowWorkspace";
import { LatencyOpsWorkspace } from "../workspaces/analytics/LatencyOpsWorkspace";
import { EventTraceWorkspace } from "../workspaces/analytics/EventTraceWorkspace";
import { StreetLiquidityWorkspace } from "../workspaces/analytics/StreetLiquidityWorkspace";
import { ExcelWorkspace } from "../workspaces/ExcelWorkspace";
import { StatusRibbon } from "./StatusRibbon";
import { CelerMark, CelnetWordmark } from "../components/CelerMark";
import { ScopeControl } from "../components/ScopeControl";
import { ScopeSwitcher } from "../components/ScopeSwitcher";
import { SavedViewsMenu } from "../components/SavedViewsMenu";
import { StudioSwitcher, CELNET_DESKS } from "../components/StudioSwitcher";
import { AuthMenu } from "../components/AuthMenu";
import { NotificationCenter } from "../components/NotificationCenter";
import { SettingsPanel } from "../components/SettingsPanel";
import { SettingsProvider } from "../settings/SettingsProvider";
import { PricingControlProvider } from "./PricingControlProvider";
import { PricingHaltBanner } from "../components/PricingHaltBanner";
import { PricingControlMenu } from "../components/PricingControlMenu";
import { SignInDialog } from "../components/SignInDialog";
import {
  buildCommands,
  CONSOLIDATED_ALIAS_ENTRIES,
  configuredLicense,
  domainAccessible,
  DOMAINS,
  firstAccessibleWorkspace,
  LICENSE_UPSELL_TITLE,
  RAIL,
  railChord,
  railForDomain,
  railRowPresentation,
  railSections,
  railState,
  resolveChord,
  workspaceDomains,
  type Command,
  type Domain,
  type RailState,
  type WorkspaceId,
} from "../lib/commands";
import { isTerminal } from "../lib/scope";
import styles from "./Shell.module.css";

/**
 * The workspace components, keyed by id, for the persistent-mount canvas.
 *
 * fe-fi-migration #6 (capstone): the duplicated FX/FI rail rows that seeded the
 * shared workspaces at a fixed lens are COLLAPSED into ONE class-parametric row
 * each. There is no longer a `rates`/`curve`/`ratesrisk`/`deals`/`ratesbook` entry
 * point pinning a lens — instead each shared workspace opens at its own default
 * lens and the trader chooses the ASSET CLASS INSIDE via the lens bar + the active
 * scope/underlier (built by #1–#4). No capability is lost: the FI curve is the
 * Market Data workspace's rates lens, FI risk its rates lens, the FI book its
 * positions lens, and the rates ticket its fixed-income product family — all
 * reachable from the single rail under a fixed-income scope/license.
 */
const WORKSPACE_VIEW: Record<WorkspaceId, () => React.ReactElement> = {
  // Ticket (Price): class-parametric — a rates instrument is priced through the
  // SAME card via its fixed-income product family; a universe-underlier drill
  // pre-targets the cross-asset spec (#3). Opens the default (FX) structure.
  ticket: TicketWorkspace,
  stream: StreamWorkspace,
  // FI Streaming: live bond + swap prices with the right-hand RFS request sidebar.
  // A Fixed-Income-only row (never under the FX tab).
  fistreaming: FiStreamingWorkspace,
  // Agg Book: the FI aggregated-book live composite view (consolidated best
  // bid/offer across a book's inbound liquidity members). Fixed-Income-only.
  aggbook: AggregatedBookWorkspace,
  // Tiering: the session→pricing-group roster is CONSOLIDATED into the "Pricing"
  // workspace as its "Tiering" tab (no standalone rail entry). This id is kept valid
  // so any deep-link (command palette / saved views) lands straight on that tab
  // within the merged surface. FI-only, `manage_pricing·FI`-gated (via the alias).
  tiering: () => <PricingGroupsWorkspace initialTab="tiering" />,
  // Risk Portfolios: the hierarchical risk-portfolio tree editor is CONSOLIDATED into
  // the Risk Dashboard as its "Portfolios" tab (no standalone rail entry). This id is
  // kept valid so any deep-link (command palette / saved views) lands straight on that
  // tab within the merged surface. FI-only.
  riskbooks: () => <RiskDashboardWorkspace initialTab="portfolios" />,
  // Risk: the consolidated FI risk surface — a tabbed shell hosting "Dashboard" (the
  // per-portfolio routed-risk roll-up + heat overview), "Portfolios" (the tree editor),
  // "Routing" (the fill→portfolio rule builder), "Acceptance" (the accept/reject rule
  // builder) and the FI position-ledger views "Positions" / "Quotes" / "Deals" (folded
  // in from the old "Book"). FI-only; each tab keeps its own gate (risk_manage /
  // manage_acceptance / view). The retired `riskbooks` / `riskrouting` / `acceptance`
  // ids deep-link straight onto their folded tab.
  riskdashboard: RiskDashboardWorkspace,
  // The FI LEDGER host ("Book") — the SAME tabbed shell mounted with its ledger tab set
  // (Positions · Quotes · Client blotter · Hedge blotter · Hedge flows). One component,
  // two tab sets: the blotters and the risk-management views share every grid, filter
  // and live-subscription behaviour, so splitting the NAVIGATION does not fork the
  // implementation. FI-only; each tab keeps its original gate.
  filedgers: () => <RiskDashboardWorkspace variant="ledgers" />,
  // Risk Routing is CONSOLIDATED into the "Risk" host as its "Routing" tab (no
  // standalone rail row). This id stays valid so any deep-link lands straight on that
  // tab within the merged surface. FI-only.
  riskrouting: () => <RiskDashboardWorkspace initialTab="routing" />,
  // Auto-Hedging (docs/hedging/AUTO-HEDGING-AND-INTERNALISATION-REQUIREMENTS.md): the
  // trader-composed EXIT-POLICY graph (reusing the risk-routing drag-and-drop
  // editor with ExitAction leaves), the warehouse-threshold config, and the live
  // hedge monitor (advisory intents + provenance + per-book band RAG). Rail-gated
  // on the narrow `hedge` capability × FI. FI-only.
  hedgeflow: HedgeFlowWorkspace,
  hedging: HedgingWorkspace,
  // Incoming-quote Acceptance is CONSOLIDATED into the "Risk" host as its "Acceptance"
  // tab (no standalone rail row). This id stays valid so any deep-link lands straight on
  // that tab within the merged surface; the tab keeps its `manage_acceptance·FI` gate.
  // FI-only.
  acceptance: () => <RiskDashboardWorkspace initialTab="acceptance" />,
  // Risk Transfer (docs/hedging/RISK-TRANSFER-REQUIREMENTS.md §9): the CONSOLIDATED FI move-
  // existing-risk surface — a tabbed shell hosting the "Risk Transfer" initiate ticket
  // (default), the "Inbox" accept/reject four-eyes counterparty tab, and the "Audit"
  // immutable provenance blotter. FI-only; the initiate + inbox TABS gate on the
  // `risk_transfer` capability, the audit tab (and the rail row) on `view`. The retired
  // `transferinbox` / `transferaudit` ids stay valid deep-links that open the merged
  // surface straight on their folded tab (via `initialTab`).
  risktransfer: RiskTransferWorkspace,
  transferinbox: () => <RiskTransferWorkspace initialTab="inbox" />,
  transferaudit: () => <RiskTransferWorkspace initialTab="audit" />,
  // Market Data: FX vol surface + FI rates curve as two lenses of ONE workspace
  // (#2). Opens the FX surface lens by default; the FI curve is the rates lens.
  surface: MarketDataWorkspace,
  // Risk: FX scenario + FI rates risk as two lenses of ONE workspace (#1). Opens
  // the FX lens by default; a rates book is risked via the fixed-income lens.
  risk: RiskWorkspace,
  // Book: Positions & Booking / Aggregate Risk / Deals as three VIEW lenses of ONE
  // workspace (#4). Opens the Aggregate-Risk lens by default; positions/booking and
  // the executed-deals blotter are the other lenses.
  book: BookWorkspace,
  quoting: QuotingWorkspace,
  xva: XvaWorkspace,
  excel: ExcelWorkspace,
  // Analytics: the cross-asset client-flow / P&L-attribution table — its own
  // top-level tab, gated on `view_analytics` (hidden without it). FI + FXO.
  clientflow: ClientFlowWorkspace,
  // Analytics: the per-stage pipeline-latency / ops table — same top-level tab
  // and `view_analytics` gate as Client Flow. Cross-asset, read-only.
  latencyops: LatencyOpsWorkspace,
  // Analytics: the street-side LP-liquidity league table — same top-level tab and
  // `view_analytics` gate. Cross-asset, read-only.
  streetliquidity: StreetLiquidityWorkspace,
  // Analytics: the per-lift Event Trace timeline — same top-level tab and
  // `view_analytics` gate. Cross-asset, read-only.
  eventtrace: EventTraceWorkspace,
  connections: ConnectionsWorkspace,
  liquidity: LiquidityWorkspace,
  admin: AdminWorkspace,
  permissions: PermissionsWorkspace,
  // Pricing: the CONSOLIDATED FI client-pricing surface — a tabbed shell hosting the
  // "Pricing Groups" drag-and-drop pipeline builder (default) and the "Tiering"
  // session→group roster. `manage_pricing·FI`-gated.
  pricinggroups: PricingGroupsWorkspace,
  // Corporate Actions: the bond CA inbox + effective-schedule viewer under the FI
  // tab. Reads on the `view·FI` floor; Confirm/Apply gate on `refdata` per-control.
  corpactions: CorporateActionsWorkspace,
  refdata: ReferenceDataWorkspace,
  studio_markets: () => <UnifiedMarketStudio />,
  studio_pricing: () => <UnifiedPricingStudio />,
  studio_distribution: () => <UnifiedDistributionStudio />,
  studio_blotter: () => <UnifiedBlotterStudio />,
  studio_risk: () => <UnifiedRiskStudio />,
  studio_policy: () => <UnifiedPolicyStudio />,
};

export function Shell(): React.ReactElement {
  const app = useApp();
  const { appearance, toggleAppearance, toggleContrast } = useAppearance();
  // The keyboard-shortcut cheatsheet overlay (bound to `?`). Shell-local UI.
  const [shortcutsOpen, setShortcutsOpen] = useState(false);
  // The searchable Help & tutorials center (opened from the header "?" affordance).
  const [helpCenterOpen, setHelpCenterOpen] = useState(false);

  // Navigation gating (single source: lib/commands.ts) is THREE-state
  // (DEC-license-gating-and-scope), now over the ONE class-parametric rail
  // (fe-fi-migration #6 — the FX/FI domain-tab split is retired). A workspace's
  // `railState` is:
  //   • HIDDEN — not reachable (no `view` on ANY class it serves / non-admin on an
  //     admin pane): an information-barrier hide.
  //   • GATED-UPSELL — reachable but the firm is licensed for NONE of its served
  //     classes: PRESENT but greyed + locked + a "license this class" upsell
  //     (discoverable, not hidden). A cross-asset row stays present while EITHER
  //     class is licensed; its unlicensed lens is gated per-lens INSIDE the pane.
  //   • PRESENT — reachable AND licensed: a normal, navigable entry.
  // The license predicate is config-driven (`VITE_CELNET_UNLICENSED`) and DEFAULTS
  // TO ALL-LICENSED, so with no config `railState` is present-or-hidden exactly as
  // the two-state — the rail is byte-identical unless a class is explicitly gated.
  // Signed out, `can` is permissive ⇒ every trading workspace is reachable.
  const licensed = useMemo(() => configuredLicense(), []);
  const stateOfWs = (r: (typeof RAIL)[number]): RailState => railState(r.id, app.auth, licensed);
  // The top-level product-DOMAIN tab three-state (fe-fi-migration re-add): a tab is
  // HIDDEN when its domain is inaccessible (admin for a non-admin; a trading class
  // the identity can't `view`), GATED-UPSELL when a trading domain's class is
  // unlicensed (present + lock + "license this class"), else PRESENT. With the
  // default all-licensed predicate this collapses to present-or-hidden, so signed
  // out both trading tabs are present and Administration is hidden.
  const domainStateOf = (d: Domain): RailState => {
    if (!domainAccessible(d, app.auth)) return "hidden";
    // Admin, Analytics and Hedging are cross-asset ops/analytics/risk-exit tabs with
    // NO license concept (`licensed` only ranges over CapabilityAsset), so they never
    // enter the gated-upsell state — only the trading domains do.
    if (
      d !== "admin" &&
      d !== "analytics" &&
      d !== "hedging" &&
      d !== "risk" &&
      !licensed(d)
    ) {
      return "gated-upsell";
    }
    return "present";
  };
  const domainTabs = DOMAINS.map((d) => ({ def: d, state: domainStateOf(d.id) })).filter(
    (t) => t.state !== "hidden",
  );

  // Per-domain last-active workspace memory (Model A): switching back to a domain
  // returns to the screen you left there, defaulting to that domain's first
  // accessible workspace. Recorded for the active domain as the workspace changes.
  const lastByDomain = useRef<Partial<Record<Domain, WorkspaceId>>>({});
  useEffect(() => {
    lastByDomain.current[app.activeDomain] = app.workspace;
  }, [app.activeDomain, app.workspace]);

  // Select a top-level domain tab. A gated/hidden tab is a no-op (the upsell lock).
  // For a PRESENT tab: flip the active domain (Model A — a shared screen keeps its
  // pane and only flips its lens), and if the current workspace is NOT in the new
  // domain, navigate to that domain's remembered (still-present) workspace or its
  // first accessible one. `setActiveDomain` runs FIRST so the composed `navigate`
  // observes the just-set domain and leaves it in place for the shared target.
  const selectDomain = (d: Domain): void => {
    if (domainStateOf(d) !== "present") return;
    app.setActiveDomain(d);
    if (workspaceDomains(app.workspace).includes(d)) return; // shared → keep the screen
    const remembered = lastByDomain.current[d];
    const target =
      remembered !== undefined && railState(remembered, app.auth, licensed) === "present"
        ? remembered
        : firstAccessibleWorkspace(app.auth, d);
    if (target) app.setWorkspace(target);
  };

  // Move keyboard focus between domain tabs (WAI-ARIA tablist roving-tabindex):
  // Arrow Left/Right wrap across the visible tabs, Home/End jump to the ends.
  const tabRefs = useRef<(HTMLButtonElement | null)[]>([]);
  const onTabKeyDown = (e: React.KeyboardEvent<HTMLButtonElement>, index: number): void => {
    const n = domainTabs.length;
    if (n === 0) return;
    let next = index;
    if (e.key === "ArrowRight") next = (index + 1) % n;
    else if (e.key === "ArrowLeft") next = (index - 1 + n) % n;
    else if (e.key === "Home") next = 0;
    else if (e.key === "End") next = n - 1;
    else return;
    e.preventDefault();
    tabRefs.current[next]?.focus();
  };

  // Shown in the rail: the rows of the ACTIVE domain that are NOT hidden (present
  // OR gated-upsell), in registry order. The tab bar selects the domain; the rail
  // shows only that domain's workspaces (cross-asset rows appear under both trading
  // tabs — Model A). The admin/ops rows are hidden for a non-admin as before.
  const navRail = railForDomain(app.activeDomain).filter((r) => stateOfWs(r) !== "hidden");
  // Fully usable (licensed + reachable): the only entries that MOUNT a pane and
  // that keyboard/command navigation may target — a wholly-unlicensed class never
  // mounts a workspace it cannot use. (Default all-licensed ⇒ usable == shown.)
  const mountRail = RAIL.filter((r) => stateOfWs(r) === "present");
  // The consolidated deep-link ALIASES (no rail row of their own — e.g. `tiering`
  // folded into "Pricing", `riskbooks` into "Risk Dashboard"): mount a hidden pane
  // for each whose HOST row is present, so a `view=<alias>` deep-link renders the
  // host workspace on its folded tab (the alias's `initialTab` variant in
  // WORKSPACE_VIEW) instead of an empty canvas. The alias gates IDENTICALLY to its
  // host, so host-present ⇒ alias mountable.
  const presentIds = new Set(mountRail.map((r) => r.id));
  const mountAliasIds: WorkspaceId[] = CONSOLIDATED_ALIAS_ENTRIES.filter(
    ([, host]) => presentIds.has(host),
  ).map(([alias]) => alias);

  const CELNET_DESK_IDS: readonly WorkspaceId[] = [
    "studio_markets",
    "studio_pricing",
    "studio_distribution",
    "studio_blotter",
    "studio_risk",
    "studio_policy",
  ];

  // One rail entry. A license-gated (entitled-but-unlicensed) row is PRESENT-BUT-
  // LOCKED (greyed + lock + upsell title, aria-disabled, no nav handler — the class
  // is discoverable but not enterable; default all-licensed ⇒ this never renders);
  // otherwise a normal navigable button carrying the active-item highlight. Extracted
  // so the grouped-section renderer maps rows without duplicating the two branches.
  const renderRailButton = (r: (typeof RAIL)[number]): React.ReactElement => {
    // Original RAIL index keeps the ⌘N hint aligned with resolveChord (which maps
    // digits against the full, admin-filtered RAIL globally, independent of the
    // grouped visual order).
    const kbd = railChord(RAIL.indexOf(r)).join("");
    // Context-correct label/subtitle for the ACTIVE domain — the shared Market Data
    // row reads "Curves" under Fixed Income (its rates lens is the curve manager) and
    // keeps its vol-surface label under FX; every other row is unchanged.
    const { label, subtitle } = railRowPresentation(r, app.activeDomain);
    if (stateOfWs(r) === "gated-upsell") {
      return (
        <button
          key={r.id}
          type="button"
          className={`${styles.railBtn} ${styles.railLocked}`}
          aria-disabled="true"
          title={LICENSE_UPSELL_TITLE}
          aria-label={`${label} — ${LICENSE_UPSELL_TITLE}`}
        >
          <span className={styles.railGlyph} aria-hidden>
            {r.glyph}
          </span>
          <span className={styles.railLabel}>{label}</span>
          <span className={styles.lockBadge} aria-hidden>
            {"🔒︎"}
          </span>
        </button>
      );
    }
    return (
      <button
        key={r.id}
        className={`${styles.railBtn} ${app.workspace === r.id ? styles.railActive : ""}`}
        onClick={() => app.setWorkspace(r.id)}
        title={subtitle ? `${label} — ${subtitle} (${kbd})` : `${label} (${kbd})`}
        aria-label={subtitle ? `${label} — ${subtitle}` : undefined}
        aria-current={app.workspace === r.id}
      >
        <span className={styles.railGlyph}>{r.glyph}</span>
        <span className={styles.railLabel}>{label}</span>
      </button>
    );
  };

  // The runnable commands, bound to live app actions — the SINGLE source the
  // palette renders and the Shell dispatches from.
  const commands = buildCommands({
    setWorkspace: app.setWorkspace,
    openPalette: () => app.setPaletteOpen(true),
    openScopeSwitcher: () => app.setScopeSwitcherOpen(true),
    showShortcuts: () => setShortcutsOpen(true),
    drillScopeDown: () => app.setScopeSwitcherOpen(true),
    resetScope: app.resetScope,
    markSurface: () => {
      app.setWorkspace("surface");
      void app.remarkSurface();
    },
    openRiskScenario: () => app.setWorkspace("risk"),
    saveView: () => app.setScopeSwitcherOpen(false),
    toggleDensity: app.toggleDensity,
    toggleAppearance,
    toggleContrast,
    canDrillScope: !isTerminal(app.scope),
  }).filter((c) => {
    // Drop workspace-jump commands (palette + ⌘N) for any workspace that is not
    // fully USABLE — entitlement-hidden OR license-gated — so no command can
    // navigate to a pane that isn't mounted; non-workspace commands pass. (Default
    // all-licensed ⇒ this is exactly the old entitlement filter.)
    if (!c.id.startsWith("ws-")) return true;
    const id = c.id.slice("ws-".length) as WorkspaceId;
    return railState(id, app.auth, licensed) === "present";
  });

  const deskCommands = useMemo<Command[]>(
    () =>
      CELNET_DESKS.map((d) => ({
        id: `desk-${d.id}`,
        title: `Desk ${d.code}: ${d.name} — ${d.desc}`,
        group: "Trading Desks",
        hint: `Desk ${d.code}`,
        run: () => app.setWorkspace(d.id),
      })),
    [app.setWorkspace],
  );

  const allCommands = useMemo(() => [...commands, ...deskCommands], [commands, deskCommands]);

  // Global keyboard grammar (single source: lib/commands.ts). The Shell resolves a
  // keydown against the registry and dispatches the matched command, so what the
  // product HONOURS is exactly what the cheatsheet ADVERTISES.
  useEffect(() => {
    const byId = new Map(allCommands.map((c) => [c.id, c]));
    const onKey = (e: KeyboardEvent) => {
      const meta = e.metaKey || e.ctrlKey;
      // `?` is a literal character inside a text field — never a command there.
      if (!meta && e.key === "?") {
        const tag = (e.target as HTMLElement | null)?.tagName;
        if (tag === "INPUT" || tag === "TEXTAREA") return;
      }
      const hit = resolveChord({ key: e.key, meta }, RAIL.length);
      if (!hit) return;
      // `?` toggles the cheatsheet (the only toggling chord); everything else runs.
      if (hit.id === "help") {
        e.preventDefault();
        setShortcutsOpen((o) => !o);
        return;
      }
      const cmd = byId.get(hit.id);
      if (cmd) {
        e.preventDefault();
        cmd.run();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [commands]);

  // The Shell self-provides settings so its settings-consuming children (the
  // header gear + NotificationCenter) resolve even when the Shell is mounted in
  // isolation (component tests render `<AppProvider><Shell/></AppProvider>`
  // directly). In the running app this nests harmlessly under the root provider
  // in main.tsx — both hydrate from the same versioned localStorage key, and the
  // inner instance shadows the outer for this subtree (idiomatic provider nesting).
  return (
    <SettingsProvider>
    <PricingControlProvider>
    <TourProvider>
    <div className={styles.shell}>
      <aside className={styles.rail} aria-label="workspaces">
        <div className={styles.brand} title="Celnet — a Celer Technologies product">
          <CelerMark size={30} className={styles.mark} title="Celnet — a Celer Technologies product" />
        </div>
        {/*
         * Grouped, scrolling rail: the active domain's visible rows (navRail — hidden
         * rows already dropped) bucketed into labelled sections (railSections). Each
         * section is a `role="group"` labelled by its sticky micro-header, so the
         * current group stays labelled while its rows scroll. A section whose rows are
         * ALL capability-hidden yields no bucket ⇒ no stray header. The nav is the
         * scroll container (brand + railFoot stay pinned); DOM/focus order follows the
         * visual section order.
         */}
        <nav className={styles.nav} aria-label="workspace sections">
          {railSections(navRail).map((group) => {
            const headerId = `rail-section-${group.section.id}`;
            return (
              <div
                key={group.section.id}
                role="group"
                aria-labelledby={headerId}
                className={styles.railGroup}
              >
                <h2 id={headerId} className={styles.railSectionHeader}>
                  {group.section.label}
                </h2>
                {group.rows.map((r) => renderRailButton(r))}
              </div>
            );
          })}
        </nav>
        <div className={styles.railFoot}>
          <button
            className={styles.railBtn}
            onClick={() => setShortcutsOpen(true)}
            title="Keyboard shortcuts (?)"
            aria-label="show keyboard shortcuts"
          >
            <span className={styles.railGlyph} aria-hidden>
              ?
            </span>
          </button>
          <button
            className={styles.railBtn}
            onClick={toggleAppearance}
            title="Toggle light/dark"
            aria-label="toggle light or dark appearance"
          >
            <span className={styles.railGlyph} aria-hidden>
              {appearance === "dark" ? "☾" : "☀"}
            </span>
          </button>
        </div>
      </aside>

      <div className={styles.main}>
        {/*
         * Firm-wide pricing kill-switch banner (Shell-level so it spans every
         * workspace). Renders null when pricing is live — its `auto` grid row then
         * collapses to 0 — and a prominent warn/danger bar when halted. Shown to
         * EVERY user (the state is broadcast firm-wide), operable only via the
         * capability-gated PricingControlMenu in the title bar.
         */}
        <PricingHaltBanner />
        {/*
         * Top-level product-DOMAIN tab bar (fe-fi-migration re-add): FX Options /
         * Fixed Income / Administration, each capability-gated. The tab selects the
         * domain LENS for the shared cross-asset screens (Ticket / Market Data /
         * Risk / Book appear under both trading tabs — Model A) and filters the rail
         * to the active domain's rows. A gated (unlicensed) trading tab is present
         * but locked (upsell); an inaccessible tab is hidden entirely.
         */}
        <div className={styles.tabBar} role="tablist" aria-label="product domains">
          {domainTabs.map((t, i) => {
            const id = t.def.id;
            const active = app.activeDomain === id;
            const gated = t.state === "gated-upsell";
            return (
              <button
                key={id}
                ref={(el) => {
                  tabRefs.current[i] = el;
                }}
                type="button"
                role="tab"
                aria-selected={active}
                aria-disabled={gated || undefined}
                tabIndex={active ? 0 : -1}
                title={gated ? LICENSE_UPSELL_TITLE : undefined}
                className={`${styles.tab} ${active ? styles.tabActive : ""} ${
                  gated ? styles.tabLocked : ""
                }`}
                onClick={() => selectDomain(id)}
                onKeyDown={(e) => onTabKeyDown(e, i)}
              >
                {t.def.label}
                {gated && (
                  <span className={styles.tabLock} aria-hidden>
                    {"🔒︎"}
                  </span>
                )}
              </button>
            );
          })}
        </div>
        <TitleBar onOpenHelp={() => setHelpCenterOpen(true)} />
        {/*
         * P0-11: every workspace stays MOUNTED; we toggle visibility rather than
         * conditionally rendering. The canvas iterates the FULL single rail (every
         * usable workspace), so switching workspaces never remounts a pane (no lost
         * Risk/Book in-progress state, no re-fired heavy effects).
         */}
        <div className={styles.canvas}>
          {/* Rail-backed panes + the consolidated deep-link alias panes */}
          {[...mountRail.map((r) => r.id), ...mountAliasIds].map((id) => {
            const View = WORKSPACE_VIEW[id];
            const active = app.workspace === id;
            return (
              <div
                key={id}
                className={`${styles.pane} ${active ? styles.paneActive : styles.paneHidden}`}
                aria-hidden={!active}
                inert={!active}
              >
                <View />
              </div>
            );
          })}
          {/* Celnet Unified Trading Desks: rendered when active */}
          {CELNET_DESK_IDS.includes(app.workspace) && (() => {
            const StudioView = WORKSPACE_VIEW[app.workspace];
            return (
              <div key={app.workspace} className={`${styles.pane} ${styles.paneActive}`}>
                <StudioView />
              </div>
            );
          })()}
        </div>
        <StatusRibbon />
      </div>

      <CommandPalette
        open={app.paletteOpen}
        commands={allCommands}
        onClose={() => app.setPaletteOpen(false)}
      />
      <ScopeSwitcher />
      <SignInDialog />
      <ShortcutsOverlay open={shortcutsOpen} onClose={() => setShortcutsOpen(false)} />
      <HelpCenter open={helpCenterOpen} onClose={() => setHelpCenterOpen(false)} />
      {/* Startup routing guard: flashes a warning when the firm has no valid default
          routed portfolio, so unmatched fills' risk is never dropped into the void. */}
      <DefaultRoutePrompt />
    </div>
    </TourProvider>
    </PricingControlProvider>
    </SettingsProvider>
  );
}

function TitleBar({ onOpenHelp }: { onOpenHelp: () => void }): React.ReactElement {
  const app = useApp();
  return (
    <header className={styles.titleBar}>
      <CelnetWordmark className={styles.wordmark} />
      {/* The ONE scope control: "what slice of the firm", terminal = underlier. It is an
          FX book-scope + grouping breadcrumb (Firm ▸ vol book / GROUP), so it is HIDDEN
          under the Fixed Income tab — FI is scoped by desk / risk portfolio, not an FX
          vol book, and the FX book-scope + grouping selector is a meaningless carryover
          there. The FX (and admin) tabs keep it unchanged. */}
      {app.activeDomain !== "fixed_income" && (
        <>
          <span className={styles.divider} aria-hidden>
            ·
          </span>
          <ScopeControl />
        </>
      )}
      <SavedViewsMenu />
      <StudioSwitcher />
      <button
        className={styles.search}
        onClick={() => app.setPaletteOpen(true)}
        title="Search / command (⌘K)"
      >
        <kbd className={styles.kbd}>⌘K</kbd>
        <span>Search / command…</span>
      </button>
      {/* Firm-wide pricing kill-switch (top-right cluster). Hidden unless the user
          holds `manage_liquidity·fixed_income`; reflects the live broadcast state. */}
      <PricingControlMenu />
      <button
        type="button"
        className={styles.toolBtn}
        onClick={onOpenHelp}
        title="Help & tutorials — search features and launch guided walkthroughs"
        aria-label="open help and tutorials"
      >
        <span className={styles.toolGlyph} aria-hidden>
          ?
        </span>
        <span>Help</span>
      </button>
      <SettingsPanel />
      <NotificationCenter />
      <AuthMenu />
    </header>
  );
}
