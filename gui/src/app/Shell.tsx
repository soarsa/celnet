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
import { createRoot } from "react-dom/client";
import { useApp } from "./AppContext";
import { CommandPalette } from "../components/CommandPalette";
import { ShortcutsOverlay } from "../components/ShortcutsOverlay";
import { useAppearance } from "../design/appearance";
import { TicketWorkspace } from "../workspaces/TicketWorkspace";
import { MarketDataWorkspace } from "../workspaces/MarketDataWorkspace";
import { OIS_STRUCTURE_ID } from "../products/ois";
import { QuotingWorkspace } from "../workspaces/QuotingWorkspace";
import { ReferenceDataWorkspace } from "../workspaces/ReferenceDataWorkspace";
import { StreamWorkspace } from "../workspaces/StreamWorkspace";
import { RiskWorkspace } from "../workspaces/RiskWorkspace";
import { XvaWorkspace } from "../workspaces/XvaWorkspace";
import { BookWorkspace } from "../workspaces/BookWorkspace";
import { ConnectionsWorkspace } from "../workspaces/ConnectionsWorkspace";
import { AdminWorkspace } from "../workspaces/AdminWorkspace";
import { PermissionsWorkspace } from "../workspaces/PermissionsWorkspace";
import { ExcelWorkspace } from "../workspaces/ExcelWorkspace";
import { StatusRibbon } from "./StatusRibbon";
import { CelerMark, CelnetWordmark } from "../components/CelerMark";
import { ScopeControl } from "../components/ScopeControl";
import { ScopeSwitcher } from "../components/ScopeSwitcher";
import { SavedViewsMenu } from "../components/SavedViewsMenu";
import { AuthMenu } from "../components/AuthMenu";
import { NotificationCenter } from "../components/NotificationCenter";
import { SimulatorPanel } from "../components/SimulatorPanel";
import { SignInDialog } from "../components/SignInDialog";
import { capabilityDenialTitle } from "../lib/capabilityMatrix";
import {
  buildCommands,
  configuredLicense,
  domainRailState,
  DOMAINS,
  domainOf,
  LICENSE_UPSELL_TITLE,
  RAIL,
  railChord,
  railState,
  resolveChord,
  type Domain,
  type RailState,
  type WorkspaceId,
} from "../lib/commands";
import { isTerminal } from "../lib/scope";
import type { CelnetTransport } from "../data/transport";
import styles from "./Shell.module.css";

/** The workspace components, keyed by id, for the persistent-mount canvas. */
const WORKSPACE_VIEW: Record<WorkspaceId, () => React.ReactElement> = {
  ticket: TicketWorkspace,
  // fe-fi-migration #3: the standalone FI PRICING silo is folded into the ONE
  // shared ticket. The `rates` rail row is now an ENTRY POINT that opens the shared
  // TicketWorkspace seeded to the fixed-income (OIS) family — a rates instrument is
  // priced through the SAME card as FX/cross-asset (calling `priceRates`), rather
  // than a separate `RatesWorkspace`. The pricing analogue of the #1 Risk / #2
  // Market-Data lens entry-points; `ticket` opens the default FX structure.
  rates: () => <TicketWorkspace initialStructure={OIS_STRUCTURE_ID} />,
  // fe-fi-migration #2: the FX/FI market-data silo is folded into the ONE
  // class-parametric MarketDataWorkspace. The `curve` rail row is now an ENTRY
  // POINT that opens the shared Market Data workspace on its Fixed-Income (rates
  // curve) lens; `surface` opens the FX (vol surface) lens. Both mount the same
  // component, so FX vol surfaces and FI curves flow through the same workflow
  // under a license lens — no FX-vs-FI split.
  curve: () => <MarketDataWorkspace initialLens="rates" />,
  // fe-fi-migration: the FI risk silo is folded into the ONE class-parametric
  // RiskWorkspace. The `ratesrisk` rail row is now an ENTRY POINT that opens the
  // shared Risk workspace on its Fixed-Income lens (`risk` opens the FX lens);
  // both mount the same component, so a rates book is risked through the same
  // workflow as FX under a fixed-income license — no FX-vs-FI split.
  ratesrisk: () => <RiskWorkspace initialLens="rates" />,
  quoting: QuotingWorkspace,
  // fe-fi-migration #4: the three book/position/blotter silos are folded into the
  // ONE unified `BookWorkspace` with a VIEW lens toggle (Positions & Booking ·
  // Aggregate Risk · Deals). Each rail row is now an ENTRY POINT that opens the
  // shared Book on the matching lens — `deals` on the executed-deals blotter,
  // `ratesbook` on the FI position ledger + booking, `book` on the aggregate-risk
  // rollup (below). All three mount the same component, so a trader's positions,
  // booking, aggregate risk and deals flow through the ONE "Book" — no FX-vs-FI
  // (or view-vs-view) split. The Book analogue of the #1/#2/#3 lens entry points.
  deals: () => <BookWorkspace initialLens="deals" />,
  ratesbook: () => <BookWorkspace initialLens="positions" />,
  refdata: ReferenceDataWorkspace,
  stream: StreamWorkspace,
  // fe-fi-migration #2: the `surface` rail row opens the shared Market Data
  // workspace on its FX vol-surface lens (the default lens); see `curve` above.
  surface: () => <MarketDataWorkspace initialLens="fx" />,
  risk: RiskWorkspace,
  xva: XvaWorkspace,
  // fe-fi-migration #4: `book` opens the unified Book on its Aggregate Risk lens
  // (the default); `ratesbook`/`deals` open the Positions and Deals lenses (above).
  book: () => <BookWorkspace initialLens="risk" />,
  connections: ConnectionsWorkspace,
  admin: AdminWorkspace,
  permissions: PermissionsWorkspace,
  excel: ExcelWorkspace,
};

export function Shell(): React.ReactElement {
  const app = useApp();
  const { appearance, toggleAppearance, toggleContrast } = useAppearance();
  // The keyboard-shortcut cheatsheet overlay (bound to `?`). Shell-local UI.
  const [shortcutsOpen, setShortcutsOpen] = useState(false);

  // Navigation gating (single source: lib/commands.ts) is now THREE-state
  // (DEC-license-gating-and-scope). A workspace's `railState` is:
  //   • HIDDEN — entitlement-deny (no `view` on the asset class / non-admin on an
  //     admin pane): an information-barrier hide, exactly as before.
  //   • GATED-UPSELL — entitled but the asset class is NOT LICENSED: PRESENT but
  //     greyed + locked + a "license this class" upsell (discoverable, not hidden).
  //   • PRESENT — entitled AND licensed: a normal, navigable entry.
  // The license predicate is config-driven (`VITE_CELNET_UNLICENSED`) and DEFAULTS
  // TO ALL-LICENSED, so with no config `railState` is present-or-hidden exactly as
  // the old two-state — the rail is byte-identical to before. Signed out, `can` is
  // permissive ⇒ every asset domain is entitled. Every entry keeps its ORIGINAL
  // rail index so the ⌘N numbers stay aligned with `resolveChord`.
  const licensed = useMemo(() => configuredLicense(), []);
  const stateOfWs = (r: (typeof RAIL)[number]): RailState => railState(r.id, app.auth, licensed);
  // Shown in the rail: everything NOT entitlement-hidden (present OR gated-upsell).
  const railShown = (r: (typeof RAIL)[number]): boolean => stateOfWs(r) !== "hidden";
  // Fully usable (licensed + entitled): the only entries that MOUNT a pane and that
  // keyboard/command navigation may target — a gated class never mounts a workspace
  // it cannot price. (Default all-licensed ⇒ usable == shown, so unchanged.)
  const railUsable = (r: (typeof RAIL)[number]): boolean => stateOfWs(r) === "present";

  // GW-tabs: the rail is split into top-level DOMAIN tabs (FX Options / Fixed
  // Income / Administration). The active domain follows the active workspace; the
  // rail BUTTONS show the active domain's shown workspaces (`navRail`, incl. gated),
  // while the persistent-mount canvas iterates every USABLE domain's workspaces
  // (`mountRail`) so switching tabs never unmounts a pane (preserves P0-11
  // persistent mount) — and a hidden/gated domain's panes are never mounted.
  const activeDomain = domainOf(app.workspace);
  const mountRail = RAIL.filter(railUsable);
  const navRail = RAIL.filter((r) => r.domain === activeDomain && railShown(r));

  // Domain tabs are three-state too: entitlement-deny (or non-admin on
  // Administration) hides the tab; an entitled-but-unlicensed asset domain shows a
  // greyed, locked tab (upsell); else present. Default all-licensed ⇒ every
  // accessible tab is present (unchanged).
  const visibleDomains = DOMAINS.map((d) => ({
    def: d,
    state: domainRailState(d.id, app.auth, licensed),
  })).filter((x) => x.state !== "hidden");

  // Per-domain memory of the last-active workspace, so re-selecting a tab returns
  // to where the trader left it (defaulting to that domain's first rail entry).
  // Shell-local UI state — kept in sync with the active workspace below.
  const [lastByDomain, setLastByDomain] = useState<Partial<Record<Domain, WorkspaceId>>>(
    () => ({ [activeDomain]: app.workspace }),
  );
  useEffect(() => {
    const d = domainOf(app.workspace);
    setLastByDomain((m) => (m[d] === app.workspace ? m : { ...m, [d]: app.workspace }));
  }, [app.workspace]);

  const firstOfDomain = (d: Domain): WorkspaceId => {
    const entry = RAIL.find((r) => r.domain === d && railUsable(r));
    return entry ? entry.id : app.workspace;
  };
  const selectDomain = (d: Domain): void => {
    // A locked (unlicensed) domain tab is an upsell affordance, not a jump: leave
    // the active workspace put. (Default all-licensed ⇒ every visible tab is
    // present, so this is a no-op guard until a class is explicitly gated.)
    if (domainRailState(d, app.auth, licensed) !== "present") return;
    app.setWorkspace(lastByDomain[d] ?? firstOfDomain(d));
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

  // Global keyboard grammar (single source: lib/commands.ts). The Shell resolves a
  // keydown against the registry and dispatches the matched command, so what the
  // product HONOURS is exactly what the cheatsheet ADVERTISES.
  useEffect(() => {
    const byId = new Map(commands.map((c) => [c.id, c]));
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

  return (
    <div className={styles.shell}>
      <aside className={styles.rail} aria-label="workspaces">
        <div className={styles.brand} title="Celnet — a Celer Technologies product">
          <CelerMark size={30} className={styles.mark} title="Celnet — a Celer Technologies product" />
        </div>
        <nav className={styles.nav}>
          {navRail.map((r) => {
            // Original RAIL index keeps the ⌘N hint aligned with resolveChord
            // (which maps digits against the full, admin-filtered RAIL globally).
            const kbd = railChord(RAIL.indexOf(r)).join("");
            // A license-gated (entitled-but-unlicensed) entry is PRESENT-BUT-LOCKED:
            // greyed, a lock badge, and a "license this class" upsell title. It is
            // `aria-disabled` and carries NO click/navigation handler — the class is
            // discoverable but not enterable until it is licensed (the server never
            // mints an unlicensed session). Default all-licensed ⇒ this never renders.
            if (stateOfWs(r) === "gated-upsell") {
              return (
                <button
                  key={r.id}
                  type="button"
                  className={`${styles.railBtn} ${styles.railLocked}`}
                  aria-disabled="true"
                  title={LICENSE_UPSELL_TITLE}
                  aria-label={`${r.label} — ${LICENSE_UPSELL_TITLE}`}
                >
                  <span className={styles.railGlyph} aria-hidden>
                    {r.glyph}
                  </span>
                  <span className={styles.railLabel}>{r.label}</span>
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
                title={`${r.label} (${kbd})`}
                aria-current={app.workspace === r.id}
              >
                <span className={styles.railGlyph}>{r.glyph}</span>
                <span className={styles.railLabel}>{r.label}</span>
              </button>
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
        <div className={styles.tabBar} role="tablist" aria-label="product domains">
          {visibleDomains.map(({ def: d, state }) => {
            const active = d.id === activeDomain;
            const gated = state === "gated-upsell";
            // A license-gated domain tab is greyed + locked + an upsell (title
            // "license this class"); the click guard in `selectDomain` keeps it
            // from navigating. Spread the lock props so present tabs stay byte-
            // identical (no undefined title/aria-disabled). Default all-licensed ⇒
            // every visible tab is present, so `gated` is never true here.
            const lockProps = gated
              ? { title: LICENSE_UPSELL_TITLE, "aria-disabled": true as const }
              : {};
            return (
              <button
                key={d.id}
                role="tab"
                aria-selected={active}
                tabIndex={active ? 0 : -1}
                className={`${styles.tab} ${active ? styles.tabActive : ""} ${gated ? styles.tabLocked : ""}`}
                onClick={() => selectDomain(d.id)}
                {...lockProps}
              >
                {d.label}
                {gated && (
                  <span className={styles.tabLock} aria-hidden>
                    {"🔒︎"}
                  </span>
                )}
              </button>
            );
          })}
        </div>
        <TitleBar />
        {/*
         * P0-11: every workspace stays MOUNTED; we toggle visibility rather than
         * conditionally rendering. The canvas iterates the FULL admin-gated rail
         * (all domains), so switching tabs/workspaces never remounts a pane (no
         * lost Risk/Book in-progress state, no re-fired heavy effects).
         */}
        <div className={styles.canvas}>
          {mountRail.map((r) => {
            const View = WORKSPACE_VIEW[r.id];
            const active = app.workspace === r.id;
            return (
              <div
                key={r.id}
                className={`${styles.pane} ${active ? styles.paneActive : styles.paneHidden}`}
                aria-hidden={!active}
                inert={!active}
              >
                <View />
              </div>
            );
          })}
        </div>
        <StatusRibbon />
      </div>

      <CommandPalette
        open={app.paletteOpen}
        commands={commands}
        onClose={() => app.setPaletteOpen(false)}
      />
      <ScopeSwitcher />
      <SignInDialog />
      <ShortcutsOverlay open={shortcutsOpen} onClose={() => setShortcutsOpen(false)} />
    </div>
  );
}

function TitleBar(): React.ReactElement {
  const app = useApp();
  // The counterparty simulator is a top-bar TOOL (not a domain tab), gated on
  // `simulate·fixed_income`: a lacking user sees it DISABLED with the denial
  // tooltip (affordance discipline — disable + explain, never silently hide).
  // Opening it pops out a SEPARATE OS window (so the main desk stays visible) into
  // which the panel is portalled — keeping it inside this React tree, so it shares
  // the SAME authenticated transport (no second auth path, no second connection).
  const [simOpen, setSimOpen] = useState(false);
  const canSimulate = app.auth.can("simulate", "fixed_income");
  return (
    <header className={styles.titleBar}>
      <CelnetWordmark className={styles.wordmark} />
      <span className={styles.divider} aria-hidden>
        ·
      </span>
      {/* The ONE scope control: "what slice of the firm", terminal = underlier. */}
      <ScopeControl />
      <SavedViewsMenu />
      <button
        className={styles.search}
        onClick={() => app.setPaletteOpen(true)}
        title="Search / command (⌘K)"
      >
        <kbd className={styles.kbd}>⌘K</kbd>
        <span>Search / command…</span>
      </button>
      <button
        type="button"
        className={styles.simBtn}
        onClick={() => setSimOpen(true)}
        disabled={!canSimulate}
        title={canSimulate ? "Open the counterparty simulator" : capabilityDenialTitle("simulate", "fixed_income")}
        aria-label="open counterparty simulator"
      >
        <span className={styles.simGlyph} aria-hidden>
          ⚗
        </span>
        <span>Simulator</span>
      </button>
      <NotificationCenter />
      <AuthMenu />
      {simOpen && <SimulatorPopout transport={app.transport} onClose={() => setSimOpen(false)} />}
    </header>
  );
}

/**
 * SimulatorPopout — opens a real separate OS window via `window.open` and mounts a
 * dedicated React root inside it hosting the {@link SimulatorPanel}. A separate
 * root (not a portal) is required so the panel's DOM events bind to the popout
 * document; the panel is handed the opener's LIVE `transport` BY REFERENCE, so it
 * injects over the SAME authenticated connection the main desk uses — no second
 * auth path, no second connection bootstrap. The window is same-origin, so cloning
 * the opener's stylesheets + cascade attributes into it renders the live theme.
 *
 * This component renders nothing into the opener tree; it manages the child window
 * imperatively and tears it down on unmount (close button, Esc, or sign-out).
 */
function SimulatorPopout({
  transport,
  onClose,
}: {
  transport: CelnetTransport;
  onClose: () => void;
}): null {
  // The latest onClose, read through a ref so the open-effect runs exactly ONCE
  // (a fresh inline onClose each render must not reopen the window).
  const onCloseRef = useRef(onClose);
  onCloseRef.current = onClose;

  useEffect(() => {
    const win = window.open(
      "",
      "celnet-simulator",
      "width=760,height=860,menubar=no,toolbar=no,location=no,status=no",
    );
    if (!win) {
      // Popup blocked — surface honestly via the opener and bail (no silent fail).
      window.alert("The simulator window was blocked. Allow pop-ups for this site and retry.");
      onCloseRef.current();
      return;
    }

    const doc = win.document;
    doc.title = "Celnet — Simulator";
    // The popout document is created blank — give its root a lang for a11y, and
    // mirror the opener's when it has one.
    doc.documentElement.lang = document.documentElement.lang || "en";
    // Carry the live theme: every cascade attribute (appearance / density /
    // contrast — all `data-*` on the root) drives the design tokens, and the
    // opener's stylesheets (CSS-module <style> in dev, <link> in prod) render it.
    // Copied generically so no one cascade axis is named in JS here.
    for (const attr of Array.from(document.documentElement.attributes)) {
      if (attr.name.startsWith("data-")) {
        doc.documentElement.setAttribute(attr.name, attr.value);
      }
    }
    for (const node of document.querySelectorAll('style, link[rel="stylesheet"]')) {
      doc.head.appendChild(node.cloneNode(true));
    }
    doc.body.style.margin = "0";

    // A dedicated root in the popout document so the panel's events are live there.
    const root = createRoot(doc.body);
    root.render(<SimulatorPanel transport={transport} onClose={() => onCloseRef.current()} />);

    // Closing the window (OS chrome) is equivalent to closing the simulator; and
    // closing the opener tab must take its child window with it.
    const handleUnload = (): void => onCloseRef.current();
    win.addEventListener("pagehide", handleUnload);
    const closeChild = (): void => win.close();
    window.addEventListener("pagehide", closeChild);

    win.focus();

    return () => {
      win.removeEventListener("pagehide", handleUnload);
      window.removeEventListener("pagehide", closeChild);
      root.unmount();
      if (!win.closed) win.close();
    };
    // Mount-once: transport is stable from app context; onClose is read via ref.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return null;
}
