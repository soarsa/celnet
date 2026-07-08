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
import { QuotingWorkspace } from "../workspaces/QuotingWorkspace";
import { ReferenceDataWorkspace } from "../workspaces/ReferenceDataWorkspace";
import { StreamWorkspace } from "../workspaces/StreamWorkspace";
import { FiStreamingWorkspace } from "../workspaces/FiStreamingWorkspace";
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
import { SettingsPanel } from "../components/SettingsPanel";
import { SettingsProvider } from "../settings/SettingsProvider";
import { SimulatorPanel } from "../components/SimulatorPanel";
import { SignInDialog } from "../components/SignInDialog";
import { capabilityDenialTitle } from "../lib/capabilityMatrix";
import {
  buildCommands,
  configuredLicense,
  domainAccessible,
  DOMAINS,
  firstAccessibleWorkspace,
  LICENSE_UPSELL_TITLE,
  RAIL,
  railChord,
  railForDomain,
  railState,
  resolveChord,
  workspaceDomains,
  type Domain,
  type RailState,
  type WorkspaceId,
} from "../lib/commands";
import { isTerminal } from "../lib/scope";
import type { CelnetTransport } from "../data/transport";
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
  connections: ConnectionsWorkspace,
  admin: AdminWorkspace,
  permissions: PermissionsWorkspace,
  refdata: ReferenceDataWorkspace,
};

export function Shell(): React.ReactElement {
  const app = useApp();
  const { appearance, toggleAppearance, toggleContrast } = useAppearance();
  // The keyboard-shortcut cheatsheet overlay (bound to `?`). Shell-local UI.
  const [shortcutsOpen, setShortcutsOpen] = useState(false);

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
    if (d !== "admin" && !licensed(d)) return "gated-upsell";
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

  // The Shell self-provides settings so its settings-consuming children (the
  // header gear + NotificationCenter) resolve even when the Shell is mounted in
  // isolation (component tests render `<AppProvider><Shell/></AppProvider>`
  // directly). In the running app this nests harmlessly under the root provider
  // in main.tsx — both hydrate from the same versioned localStorage key, and the
  // inner instance shadows the outer for this subtree (idiomatic provider nesting).
  return (
    <SettingsProvider>
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
        <TitleBar />
        {/*
         * P0-11: every workspace stays MOUNTED; we toggle visibility rather than
         * conditionally rendering. The canvas iterates the FULL single rail (every
         * usable workspace), so switching workspaces never remounts a pane (no lost
         * Risk/Book in-progress state, no re-fired heavy effects).
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
    </SettingsProvider>
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
      <SettingsPanel />
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
