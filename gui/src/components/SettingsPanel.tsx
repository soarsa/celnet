/**
 * SettingsPanel — the trader's preferences popover, opened from a header gear. It
 * edits the persisted {@link AppSettings} (via {@link useSettings}) across four
 * opinionated sections: Alerts (master), Sound (enable + volume), Thresholds
 * (min-notional, with a live compact echo), and Notifications (desktop growl +
 * auto-clear). Every control writes immediately through `update` — there is no
 * separate save step, so a reload restores exactly what the trader set.
 *
 * Accessible by construction: the popover is a labelled `role="dialog"` with
 * `aria-modal`; opening focuses the panel and closing returns focus to the gear;
 * Escape and an outside click both close it. Nested switches are real
 * `role="switch"` buttons with `aria-checked`.
 */

import { useCallback, useEffect, useId, useRef, useState } from "react";
import { useSettings } from "../hooks/useSettings";
import { fmtCompact } from "../lib/format";
import styles from "./SettingsPanel.module.css";

/** An accessible on/off switch driven by a boolean + a setter. */
function Switch({
  checked,
  onChange,
  label,
  disabled = false,
}: {
  checked: boolean;
  onChange: (next: boolean) => void;
  label: string;
  disabled?: boolean;
}): React.ReactElement {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={label}
      disabled={disabled}
      className={styles.switch}
      onClick={() => onChange(!checked)}
    >
      <span className={styles.switchTrack} aria-hidden>
        <span className={styles.switchThumb} />
      </span>
    </button>
  );
}

/** The live browser notification permission, read directly (it is global/live). */
function notificationPermission(): NotificationPermission | "unsupported" {
  if (typeof window === "undefined" || !("Notification" in window)) return "unsupported";
  return window.Notification.permission;
}

/** A short human status for the desktop-permission state. */
function permissionHint(p: NotificationPermission | "unsupported"): string {
  switch (p) {
    case "unsupported":
      return "Not supported in this browser";
    case "denied":
      return "Blocked — allow in browser settings";
    case "default":
      return "Permission not yet granted";
    case "granted":
      return "Permission granted";
  }
}

export function SettingsPanel(): React.ReactElement {
  const { settings, update } = useSettings();
  const [open, setOpen] = useState(false);
  const [permission, setPermission] = useState(() => notificationPermission());

  const gearRef = useRef<HTMLButtonElement>(null);
  const panelRef = useRef<HTMLDivElement>(null);
  const titleId = useId();

  const close = useCallback(() => setOpen(false), []);

  // On open: focus the panel. On close: return focus to the gear (only when it
  // was our open that is closing, tracked by a ref so the mount does not steal focus).
  const wasOpenRef = useRef(false);
  useEffect(() => {
    if (open) {
      setPermission(notificationPermission());
      panelRef.current?.focus();
      wasOpenRef.current = true;
    } else if (wasOpenRef.current) {
      gearRef.current?.focus();
      wasOpenRef.current = false;
    }
  }, [open]);

  // Escape-to-close + outside-click-to-close while open.
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent): void => {
      if (e.key === "Escape") {
        e.stopPropagation();
        close();
      }
    };
    const onDown = (e: MouseEvent): void => {
      const t = e.target as Node | null;
      if (
        t &&
        !panelRef.current?.contains(t) &&
        !gearRef.current?.contains(t)
      ) {
        close();
      }
    };
    document.addEventListener("keydown", onKey);
    document.addEventListener("mousedown", onDown);
    return () => {
      document.removeEventListener("keydown", onKey);
      document.removeEventListener("mousedown", onDown);
    };
  }, [open, close]);

  // Toggling desktop growl ON while the browser permission is still undecided
  // requests it (this click is the required user gesture). Guarded; never throws.
  const onToggleGrowl = useCallback(
    (next: boolean) => {
      update({ growlEnabled: next });
      if (next && notificationPermission() === "default") {
        try {
          const r = window.Notification.requestPermission();
          if (r && typeof r.then === "function") {
            r.then((p) => setPermission(p)).catch(() => {});
          }
        } catch {
          /* legacy / blocked — the NC bell toggle remains the fallback path */
        }
      }
    },
    [update],
  );

  const soundsOff = !settings.soundsEnabled || !settings.alertsEnabled;

  return (
    <div className={styles.root}>
      <button
        ref={gearRef}
        type="button"
        className={styles.gear}
        aria-label="Settings"
        aria-expanded={open}
        aria-haspopup="dialog"
        onClick={() => setOpen((o) => !o)}
      >
        <span className={styles.gearGlyph} aria-hidden>
          ⚙
        </span>
      </button>

      {open && (
        <div
          ref={panelRef}
          className={styles.panel}
          role="dialog"
          aria-modal="true"
          aria-labelledby={titleId}
          tabIndex={-1}
        >
          <div className={styles.panelHead}>
            <h2 id={titleId} className={styles.panelTitle}>
              Settings
            </h2>
            <button
              type="button"
              className={styles.closeBtn}
              aria-label="Close settings"
              onClick={close}
            >
              <span aria-hidden>×</span>
            </button>
          </div>

          {/* --- Alerts --- */}
          <section className={styles.section} aria-labelledby={`${titleId}-alerts`}>
            <h3 id={`${titleId}-alerts`} className={styles.sectionTitle}>
              Alerts
            </h3>
            <div className={styles.row}>
              <span className={styles.rowLabel}>In-app alerts</span>
              <Switch
                checked={settings.alertsEnabled}
                onChange={(v) => update({ alertsEnabled: v })}
                label="In-app alerts"
              />
            </div>
            <p className={styles.hint}>
              Master switch — off silences the bell, toasts, sound, and desktop alerts.
            </p>
          </section>

          {/* --- Sound --- */}
          <section className={styles.section} aria-labelledby={`${titleId}-sound`}>
            <h3 id={`${titleId}-sound`} className={styles.sectionTitle}>
              Sound
            </h3>
            <div className={styles.row}>
              <span className={styles.rowLabel}>Sound cue</span>
              <Switch
                checked={settings.soundsEnabled}
                onChange={(v) => update({ soundsEnabled: v })}
                label="Sound cue"
                disabled={!settings.alertsEnabled}
              />
            </div>
            <div className={styles.row}>
              <label className={styles.rowLabel} htmlFor={`${titleId}-vol`}>
                Volume
              </label>
              <span className={styles.sliderWrap}>
                <input
                  id={`${titleId}-vol`}
                  type="range"
                  min={0}
                  max={100}
                  step={1}
                  value={settings.volume}
                  disabled={soundsOff}
                  className={styles.slider}
                  onChange={(e) => update({ volume: Number(e.target.value) })}
                />
                <span className={styles.sliderValue}>{settings.volume}%</span>
              </span>
            </div>
          </section>

          {/* --- Thresholds --- */}
          <section className={styles.section} aria-labelledby={`${titleId}-thresh`}>
            <h3 id={`${titleId}-thresh`} className={styles.sectionTitle}>
              Thresholds
            </h3>
            <div className={styles.row}>
              <label className={styles.rowLabel} htmlFor={`${titleId}-minqty`}>
                Min notional
              </label>
              <span className={styles.numWrap}>
                <input
                  id={`${titleId}-minqty`}
                  type="number"
                  min={0}
                  step={1000000}
                  value={settings.minQty}
                  className={styles.numInput}
                  onChange={(e) => {
                    const v = Number(e.target.value);
                    update({ minQty: Number.isFinite(v) && v > 0 ? v : 0 });
                  }}
                />
                <span className={styles.numEcho}>≥ {fmtCompact(settings.minQty)}</span>
              </span>
            </div>
            <p className={styles.hint}>
              Requests below this size raise no alert, sound, or desktop notification.
            </p>
          </section>

          {/* --- Notifications --- */}
          <section className={styles.section} aria-labelledby={`${titleId}-notif`}>
            <h3 id={`${titleId}-notif`} className={styles.sectionTitle}>
              Notifications
            </h3>
            <div className={styles.row}>
              <span className={styles.rowLabel}>Desktop alerts</span>
              <Switch
                checked={settings.growlEnabled}
                onChange={onToggleGrowl}
                label="Desktop alerts"
                disabled={permission === "unsupported" || permission === "denied"}
              />
            </div>
            <p className={styles.hint}>{permissionHint(permission)}</p>
            <div className={styles.row}>
              <span className={styles.rowLabel}>Auto-clear completed</span>
              <Switch
                checked={settings.autoClearCompleted}
                onChange={(v) => update({ autoClearCompleted: v })}
                label="Auto-clear completed"
              />
            </div>
            <div className={styles.row}>
              <label className={styles.rowLabel} htmlFor={`${titleId}-ttl`}>
                Clear after (s)
              </label>
              <span className={styles.numWrap}>
                <input
                  id={`${titleId}-ttl`}
                  type="number"
                  min={0}
                  step={5}
                  value={settings.autoClearTtlSeconds}
                  disabled={!settings.autoClearCompleted}
                  className={styles.numInput}
                  onChange={(e) => {
                    const v = Number(e.target.value);
                    update({ autoClearTtlSeconds: Number.isFinite(v) && v > 0 ? v : 0 });
                  }}
                />
              </span>
            </div>
          </section>
        </div>
      )}
    </div>
  );
}
