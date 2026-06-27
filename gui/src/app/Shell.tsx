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

import { Fragment, useEffect, useState } from "react";
import { useApp } from "./AppContext";
import { CommandPalette } from "../components/CommandPalette";
import { ShortcutsOverlay } from "../components/ShortcutsOverlay";
import { useAppearance } from "../design/appearance";
import { TicketWorkspace } from "../workspaces/TicketWorkspace";
import { RatesWorkspace } from "../workspaces/RatesWorkspace";
import { CurveWorkspace } from "../workspaces/CurveWorkspace";
import { StreamWorkspace } from "../workspaces/StreamWorkspace";
import { SurfaceWorkspace } from "../workspaces/SurfaceWorkspace";
import { RiskWorkspace } from "../workspaces/RiskWorkspace";
import { BookWorkspace } from "../workspaces/BookWorkspace";
import { ConnectionsWorkspace } from "../workspaces/ConnectionsWorkspace";
import { AdminWorkspace } from "../workspaces/AdminWorkspace";
import { ExcelWorkspace } from "../workspaces/ExcelWorkspace";
import { StatusRibbon } from "./StatusRibbon";
import { CelerMark, CelnetWordmark } from "../components/CelerMark";
import { ScopeControl } from "../components/ScopeControl";
import { ScopeSwitcher } from "../components/ScopeSwitcher";
import { SavedViewsMenu } from "../components/SavedViewsMenu";
import { AuthMenu } from "../components/AuthMenu";
import { SignInDialog } from "../components/SignInDialog";
import { buildCommands, RAIL, railChord, resolveChord, type WorkspaceId } from "../lib/commands";
import { isTerminal } from "../lib/scope";
import styles from "./Shell.module.css";

/** The workspace components, keyed by id, for the persistent-mount canvas. */
const WORKSPACE_VIEW: Record<WorkspaceId, () => React.ReactElement> = {
  ticket: TicketWorkspace,
  rates: RatesWorkspace,
  curve: CurveWorkspace,
  stream: StreamWorkspace,
  surface: SurfaceWorkspace,
  risk: RiskWorkspace,
  book: BookWorkspace,
  connections: ConnectionsWorkspace,
  admin: AdminWorkspace,
  excel: ExcelWorkspace,
};

export function Shell(): React.ReactElement {
  const app = useApp();
  const { appearance, toggleAppearance, toggleContrast } = useAppearance();
  // The keyboard-shortcut cheatsheet overlay (bound to `?`). Shell-local UI.
  const [shortcutsOpen, setShortcutsOpen] = useState(false);

  // The Administration group (Connections + Admin) is admin-only: a non-admin sees
  // neither rail button nor pane, and their workspace-jumps are dropped from the
  // palette / keyboard. Each visible entry keeps its ORIGINAL rail index so the ⌘N
  // numbers stay aligned with `resolveChord` (which maps digits against the full
  // RAIL); the admin-group chords simply resolve to now-absent commands and are inert.
  const isAdmin = app.auth.isAdmin;
  const rail = isAdmin ? RAIL : RAIL.filter((r) => r.group !== "administration");

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
  }).filter((c) => isAdmin || (c.id !== "ws-connections" && c.id !== "ws-admin"));

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
          {rail.map((r, i) => {
            // Original RAIL index keeps the ⌘N hint aligned with resolveChord.
            const kbd = railChord(RAIL.indexOf(r)).join("");
            // The Administration group (Connections + Admin) is set off under a
            // labelled hairline divider at the foot of the rail.
            const startsAdmin =
              r.group === "administration" && rail[i - 1]?.group !== "administration";
            return (
              <Fragment key={r.id}>
                {startsAdmin && (
                  <span className={styles.railGroupLabel} aria-hidden>
                    Admin
                  </span>
                )}
                <button
                  className={`${styles.railBtn} ${app.workspace === r.id ? styles.railActive : ""}`}
                  onClick={() => app.setWorkspace(r.id)}
                  title={`${r.label} (${kbd})`}
                  aria-current={app.workspace === r.id}
                >
                  <span className={styles.railGlyph}>{r.glyph}</span>
                  <span className={styles.railLabel}>{r.label}</span>
                </button>
              </Fragment>
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
        <TitleBar />
        {/*
         * P0-11: every workspace stays MOUNTED; we toggle visibility rather than
         * conditionally rendering. Switching no longer remounts (no lost Risk/Book
         * in-progress state, no re-fired heavy effects).
         */}
        <div className={styles.canvas}>
          {rail.map((r) => {
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
      <AuthMenu />
    </header>
  );
}
