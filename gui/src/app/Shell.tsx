/**
 * Shell — the single-window, multi-pane workspace (GUI-DESIGN §2): a persistent
 * left rail of workspaces, a title + command bar, the active workspace canvas,
 * and a slim status ribbon carrying stream health + the global clock. Workspace
 * switches cross-fade with a small parallax (depth cue, §3.4).
 */

import { useEffect } from "react";
import { useApp, type WorkspaceId } from "./AppContext";
import { CommandPalette, type Command } from "../components/CommandPalette";
import { useAppearance } from "../design/appearance";
import { TicketWorkspace } from "../workspaces/TicketWorkspace";
import { StreamWorkspace } from "../workspaces/StreamWorkspace";
import { SurfaceWorkspace } from "../workspaces/SurfaceWorkspace";
import { RiskWorkspace } from "../workspaces/RiskWorkspace";
import { BookWorkspace } from "../workspaces/BookWorkspace";
import { StatusRibbon } from "./StatusRibbon";
import { CelerMark, CelerLockup } from "../components/CelerMark";
import { PairStrip } from "../components/PairStrip";
import { PAIRS, strategyInstrument } from "../data/seed";
import styles from "./Shell.module.css";

const RAIL: { id: WorkspaceId; glyph: string; label: string; kbd: string }[] = [
  { id: "ticket", glyph: "⌁", label: "Ticket", kbd: "⌘1" },
  { id: "stream", glyph: "≋", label: "Stream", kbd: "⌘2" },
  { id: "surface", glyph: "◷", label: "Surface", kbd: "⌘3" },
  { id: "risk", glyph: "⊞", label: "Risk", kbd: "⌘4" },
  { id: "book", glyph: "Σ", label: "Book", kbd: "⌘5" },
];

export function Shell(): React.ReactElement {
  const app = useApp();
  const { appearance, contrast, toggleAppearance, toggleContrast } = useAppearance();

  // Global keyboard grammar: ⌘K palette, ⌘1..4 workspaces, ⌘P pair switch.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const meta = e.metaKey || e.ctrlKey;
      if (meta && e.key.toLowerCase() === "k") {
        e.preventDefault();
        app.setPaletteOpen(true);
      } else if (meta && e.key >= "1" && e.key <= "5") {
        e.preventDefault();
        app.setWorkspace(RAIL[Number(e.key) - 1]!.id);
      } else if (meta && e.key.toLowerCase() === "p") {
        e.preventDefault();
        app.setPaletteOpen(true);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [app]);

  const commands: Command[] = [
    ...RAIL.map((r) => ({
      id: `ws-${r.id}`,
      title: `Go to ${r.label}`,
      hint: r.kbd,
      group: "Workspace",
      run: () => app.setWorkspace(r.id),
    })),
    ...PAIRS.map((p) => ({
      id: `pair-${p.pair.base}${p.pair.quote}`,
      title: `${p.pair.base}/${p.pair.quote}`,
      hint: `spot ${p.market.spot}`,
      group: "Pair",
      run: () => app.setPair(p.pair),
    })),
    {
      id: "mark-surface",
      title: "Mark surface",
      hint: "recalibrate",
      group: "Action",
      run: () => {
        app.setWorkspace("surface");
        void app.remarkSurface();
      },
    },
    {
      id: "stream-rr",
      title: `Stream ${app.pairCtx.pair.base}/${app.pairCtx.pair.quote} 1M 25Δ RR`,
      hint: "add to blotter",
      group: "Action",
      run: () => {
        // Promotes a structure into the live RFS blotter — the exact runtime
        // subscription path. The new line materializes and ticks immediately.
        app.stream.subscribe(
          strategyInstrument(app.pairCtx.pair, 30 / 365, "RISK_REVERSAL", 10),
          app.conventions,
          `${app.pairCtx.pair.base}/${app.pairCtx.pair.quote} 25Δ RR`,
        );
        app.setWorkspace("stream");
      },
    },
    {
      id: "stream-strangle",
      title: `Stream ${app.pairCtx.pair.base}/${app.pairCtx.pair.quote} 2M 10Δ strangle`,
      hint: "add to blotter",
      group: "Action",
      run: () => {
        app.stream.subscribe(
          strategyInstrument(app.pairCtx.pair, 60 / 365, "STRANGLE", 10),
          app.conventions,
          `${app.pairCtx.pair.base}/${app.pairCtx.pair.quote} 10Δ strangle`,
        );
        app.setWorkspace("stream");
      },
    },
    {
      id: "risk-scenario",
      title: "Open risk scenario",
      hint: "spot × vol shock grid",
      group: "Action",
      run: () => app.setWorkspace("risk"),
    },
    {
      id: "toggle-appearance",
      title: appearance === "dark" ? "Switch to Light" : "Switch to Dark",
      group: "Action",
      run: toggleAppearance,
    },
    {
      id: "toggle-contrast",
      title: contrast === "high" ? "Normal contrast" : "Increase contrast",
      group: "Action",
      run: toggleContrast,
    },
  ];

  return (
    <div className={styles.shell}>
      <aside className={styles.rail} aria-label="workspaces">
        <div className={styles.brand} title="Celnet · a Celer product">
          <CelerMark size={30} className={styles.mark} title="Celer" />
        </div>
        <nav className={styles.nav}>
          {RAIL.map((r) => (
            <button
              key={r.id}
              className={`${styles.railBtn} ${app.workspace === r.id ? styles.railActive : ""}`}
              onClick={() => app.setWorkspace(r.id)}
              title={`${r.label} (${r.kbd})`}
              aria-current={app.workspace === r.id}
            >
              <span className={styles.railGlyph}>{r.glyph}</span>
              <span className={styles.railLabel}>{r.label}</span>
            </button>
          ))}
        </nav>
        <div className={styles.railFoot}>
          <button
            className={styles.railBtn}
            onClick={toggleAppearance}
            title="Toggle light/dark"
          >
            <span className={styles.railGlyph}>{appearance === "dark" ? "☾" : "☀"}</span>
          </button>
        </div>
      </aside>

      <div className={styles.main}>
        <TitleBar />
        <PairStrip />
        <div className={styles.canvas} key={app.workspace}>
          {app.workspace === "ticket" && <TicketWorkspace />}
          {app.workspace === "stream" && <StreamWorkspace />}
          {app.workspace === "surface" && <SurfaceWorkspace />}
          {app.workspace === "risk" && <RiskWorkspace />}
          {app.workspace === "book" && <BookWorkspace />}
        </div>
        <StatusRibbon />
      </div>

      <CommandPalette
        open={app.paletteOpen}
        commands={commands}
        onClose={() => app.setPaletteOpen(false)}
      />
    </div>
  );
}

function TitleBar(): React.ReactElement {
  const app = useApp();
  return (
    <header className={styles.titleBar}>
      <div className={styles.trafficLights} aria-hidden>
        <span />
        <span />
        <span />
      </div>
      <CelerLockup size={22} className={styles.lockup} />
      <span className={styles.divider} aria-hidden>
        ·
      </span>
      <button
        className={styles.pairSwitch}
        onClick={() => app.setPaletteOpen(true)}
        title="Switch pair (⌘P)"
      >
        <span className={`num ${styles.pair}`}>
          {app.pairCtx.pair.base}/{app.pairCtx.pair.quote}
        </span>
        <span className={styles.caret}>▾</span>
      </button>
      <button
        className={styles.search}
        onClick={() => app.setPaletteOpen(true)}
        title="Search / command (⌘K)"
      >
        <kbd className={styles.kbd}>⌘K</kbd>
        <span>Search / command…</span>
      </button>
    </header>
  );
}
