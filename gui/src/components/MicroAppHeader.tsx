/**
 * MicroAppHeader — Info-dense, streamlined header for CelNet Micro-Apps
 * running within DeskModal windows, tearouts, or tiled workspaces.
 *
 * Implements:
 * - App/Desk branding (e.g. Desk 02 • Options Pricing)
 * - FDC3 User Channel Selector (Red, Green, Blue, etc.)
 * - Scope/Underlier selector badge
 * - Multi-Instance badge & pop-out actions
 * - Latency & Density controls
 */

import { useState, useRef, useEffect, useCallback } from "react";
import { useApp } from "../app/AppContext";
import { CELNET_DESKS } from "./StudioSwitcher";
import {
  getFdc3Agent,
  FDC3_USER_CHANNELS,
  channelIdToMetadata,
  normalizeChannelId,
} from "../lib/fdc3";
import { getInstanceId } from "../lib/deskPlatform";
import styles from "./MicroAppHeader.module.css";

export function MicroAppHeader(): React.ReactElement {
  const app = useApp();
  const [channelOpen, setChannelOpen] = useState(false);
  const [activeChannelId, setActiveChannelId] = useState<string>("fdc3.channel.4"); // default green
  const channelRef = useRef<HTMLDivElement>(null);

  const activeDesk = CELNET_DESKS.find((d) => d.id === app.workspace) || {
    id: app.workspace,
    code: "00",
    name: "CelNet Trading App",
    desc: "Institutional Trading Workbench",
  };

  const instanceId = getInstanceId();
  const leaf = app.underlier.label;

  // Sync active FDC3 channel
  useEffect(() => {
    let unmounted = false;
    const agent = getFdc3Agent();
    agent.getCurrentChannel().then((ch) => {
      if (!unmounted && ch) {
        setActiveChannelId(ch.id);
      }
    });

    const onStorage = (e: StorageEvent) => {
      if (e.key === "celnet:fdc3:channel" && e.newValue) {
        setActiveChannelId(e.newValue);
      }
    };
    window.addEventListener("storage", onStorage);
    return () => {
      unmounted = true;
      window.removeEventListener("storage", onStorage);
    };
  }, []);

  // Close dropdown on outside click
  useEffect(() => {
    function onClickOutside(e: MouseEvent) {
      if (channelRef.current && !channelRef.current.contains(e.target as Node)) {
        setChannelOpen(false);
      }
    }
    window.addEventListener("mousedown", onClickOutside);
    return () => window.removeEventListener("mousedown", onClickOutside);
  }, []);

  const handleSelectChannel = useCallback((channelId: string) => {
    const agent = getFdc3Agent();
    void agent.joinUserChannel(channelId);
    setActiveChannelId(normalizeChannelId(channelId));
    setChannelOpen(false);
  }, []);

  const handleDuplicateInstance = useCallback(() => {
    // Generate new instance ID
    const nextInst = `inst-${Date.now().toString(36).slice(-4)}`;
    const url = new URL(window.location.href);
    url.searchParams.set("instanceId", nextInst);
    window.open(url.toString(), "_blank", "width=1200,height=800,menubar=no,toolbar=no");
  }, []);

  const channelMeta = channelIdToMetadata(activeChannelId);

  return (
    <header className={styles.header} role="banner" aria-label="Micro-App Toolbar">
      <div className={styles.leftSection}>
        <div className={styles.deskBadge}>
          <span>Desk {activeDesk.code}</span>
        </div>
        <span className={styles.deskTitle}>{activeDesk.name}</span>

        <div className={styles.divider} />

        {/* FDC3 User Channel Selector */}
        <div className={styles.channelContainer} ref={channelRef}>
          <button
            type="button"
            className={styles.channelPill}
            onClick={() => setChannelOpen((o) => !o)}
            title={`FDC3 Channel: ${channelMeta.name} — Click to switch linking channel`}
            aria-label={`FDC3 Channel: ${channelMeta.name}`}
            aria-expanded={channelOpen}
          >
            <span
              className={styles.channelDot}
              style={{ backgroundColor: channelMeta.color }}
            />
            <span>{channelMeta.name}</span>
          </button>

          {channelOpen && (
            <div className={styles.channelDropdown} role="menu">
              {FDC3_USER_CHANNELS.map((ch) => {
                const isSelected = normalizeChannelId(ch.id) === normalizeChannelId(activeChannelId);
                return (
                  <button
                    key={ch.id}
                    type="button"
                    role="menuitem"
                    className={`${styles.channelOption} ${isSelected ? styles.channelOptionActive : ""}`}
                    onClick={() => handleSelectChannel(ch.id)}
                  >
                    <span
                      className={styles.channelDot}
                      style={{ backgroundColor: ch.color }}
                    />
                    <span>{ch.name}</span>
                  </button>
                );
              })}
            </div>
          )}
        </div>

        <div className={styles.divider} />

        {/* Active Instrument / Underlier */}
        <button
          type="button"
          className={styles.instrumentBtn}
          onClick={() => app.setScopeSwitcherOpen(true)}
          title="Active Instrument — Click to switch currency pair or security"
        >
          <span>{leaf || "EUR/USD"}</span>
          <span style={{ fontSize: 9, opacity: 0.7 }}>▾</span>
        </button>
      </div>

      <div className={styles.rightSection}>
        {instanceId !== "default" && (
          <span className={styles.instanceBadge} title={`Instance: ${instanceId}`}>
            #{instanceId}
          </span>
        )}

        <div className={styles.latencyPill} title="Zero-copy IPC / WebSocket pricing engine active">
          <span className={styles.latencyDot} />
          <span>LIVE</span>
        </div>

        <button
          type="button"
          className={styles.actionBtn}
          onClick={app.toggleDensity}
          title={`Density: ${app.density} — Click to toggle`}
        >
          {app.density === "compact" ? "⇲" : "⇱"}
        </button>

        <button
          type="button"
          className={styles.actionBtn}
          onClick={handleDuplicateInstance}
          title="Open New Instance / Popout Window"
        >
          ⧉
        </button>
      </div>
    </header>
  );
}
