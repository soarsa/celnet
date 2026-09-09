/**
 * HedgingHost — ONE rail row for the whole hedging story.
 *
 * Before this, a trader chasing a single hedge visited up to five destinations, and
 * three of them rendered the same table:
 *
 *   • Hedging → "Hedge Flow"      — bucket board + flow strip, WITH the ledger embedded
 *   • Hedging → "Hedging Rules"   — the exit-policy graph
 *   • Book    → "Hedge flows"     — the live engine monitor (a DIFFERENT screen from
 *                                   "Hedge Flow" above, despite the near-identical name)
 *   • Book    → "Hedge blotter"   — the same ledger again
 *   • Deals   → hedge lens        — and again
 *
 * The split was by IMPLEMENTATION, not by the question a desk asks. A hedge has one
 * life — risk builds, the engine decides, an order goes to the street, a fill lands,
 * a residual remains — and the tabs here follow that life instead of cutting across
 * it:
 *
 *   Flow     — what is filling, how far the exit got, where the risk went
 *   Monitor  — the live engine: mode, kill-switch, standing suggestions, attention
 *   Blotter  — the executed-hedge ledger. ONE copy, reached from wherever asked
 *   Rules    — how hedging is configured
 *
 * The Flow board hands its drill-downs to the Blotter tab rather than embedding a
 * second ledger, so selecting "Hedged externally" on the board and opening the
 * blotter directly land on the same rows.
 *
 * Each tab keeps the gate its surface already carried, so this changes only WHERE a
 * screen is reached, never WHO can reach it.
 */
import { useState } from "react";

import { useApp } from "../../app/AppContext";
import type { CapabilityAction } from "../../data/contract";
import { HedgeDealsView, type HedgeLeg } from "../HedgeDealsView";
import { HedgeFlowWorkspace } from "../HedgeFlowWorkspace";
import { HedgeMonitor } from "./HedgeMonitor";
import { HedgingWorkspace } from "./HedgingWorkspace";
import styles from "./HedgingHost.module.css";

/** The tabs of the hedging host, in lifecycle order. */
export type HedgingHostTab = "flow" | "monitor" | "blotter" | "rules";

/**
 * Every tab requires `hedge` × FI — the same gate the rail row carries.
 *
 * The Blotter was briefly listed on the `view` floor here, on the reasoning that reading
 * what the desk hedged is a lesser permission than running the engine. That reasoning is
 * defensible but it is NOT what the code does: `HedgeDealsView` gates itself on `hedge`
 * (server-enforced on `listHedgeProvenance`), so a `view`-only identity would have been
 * shown a tab and then a denial inside it. A tab strip that offers a door to a locked
 * room is worse than one that does not show the door.
 *
 * If the hedge ledger should genuinely sit on the `view` floor, that is a change to the
 * capability the RPC demands, not a relabelling of this tab.
 */
const TABS: readonly { tab: HedgingHostTab; label: string; cap: CapabilityAction }[] = [
  { tab: "flow", label: "Flow", cap: "hedge" },
  { tab: "monitor", label: "Monitor", cap: "hedge" },
  { tab: "blotter", label: "Blotter", cap: "hedge" },
  { tab: "rules", label: "Rules", cap: "hedge" },
];

interface HedgingHostProps {
  /**
   * The tab to open on. Drives the retired `hedging` deep-link, which now resolves
   * here on the Rules tab rather than to a rail row of its own.
   */
  initialTab?: HedgingHostTab;
}

export function HedgingHost({ initialTab = "flow" }: HedgingHostProps): React.ReactElement {
  const { auth } = useApp();
  const [tab, setTab] = useState<HedgingHostTab>(initialTab);
  /**
   * The ledger filter the Flow board handed over, or `null` for the ledger's own view.
   *
   * Held HERE rather than inside the blotter so the hand-off survives the tab switch:
   * the board is unmounted the moment the Blotter tab takes over, and a filter owned
   * by the unmounted component would be gone before the ledger could read it.
   */
  const [legFilter, setLegFilter] = useState<HedgeLeg | null>(null);

  const visibleTabs = TABS.filter((t) => auth.can(t.cap, "fixed_income"));
  // Clamp to a VISIBLE tab, so an identity that cannot view the requested one lands on
  // the first it can rather than an empty pane.
  const activeTab: HedgingHostTab = visibleTabs.some((t) => t.tab === tab)
    ? tab
    : (visibleTabs[0]?.tab ?? "flow");

  if (visibleTabs.length === 0) {
    return (
      <div className={styles.shell} data-testid="hedging-host">
        <p className={styles.denied} data-testid="hedging-denied">
          Hedging needs the <strong>hedge</strong> capability on Fixed Income. The rail
          hides this row without it; you have reached it by link.
        </p>
      </div>
    );
  }

  return (
    <div className={styles.shell} data-testid="hedging-host">
      <div className={styles.tabBar} role="group" aria-label="hedging view">
        {visibleTabs.map((t) => (
          <button
            key={t.tab}
            type="button"
            className={`${styles.tabBtn} ${activeTab === t.tab ? styles.tabBtnActive : ""}`}
            aria-pressed={activeTab === t.tab}
            data-testid={`hedging-tab-${t.tab}`}
            onClick={() => {
              setTab(t.tab);
              // Arriving at the blotter under your own steam shows the WHOLE ledger; a
              // stale filter from an earlier drill would silently hide rows the trader
              // never asked to hide.
              if (t.tab === "blotter") setLegFilter(null);
            }}
          >
            {t.label}
          </button>
        ))}
      </div>

      <div className={styles.tabPanel}>
        {activeTab === "flow" ? (
          <HedgeFlowWorkspace
            onDrill={(leg) => {
              setLegFilter(leg);
              setTab("blotter");
            }}
          />
        ) : activeTab === "monitor" ? (
          <HedgeMonitor />
        ) : activeTab === "blotter" ? (
          <HedgeDealsView legFilter={legFilter} />
        ) : (
          <HedgingWorkspace />
        )}
      </div>
    </div>
  );
}
