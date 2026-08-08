/**
 * SettingsPanel — the trader's preferences, opened from a header gear into a
 * centered popup modal. It edits the persisted {@link AppSettings} (via
 * {@link useSettings}) across opinionated sections: Alerts (master), Sound
 * (enable + volume), Thresholds (min-notional, with a live compact echo),
 * Notifications (desktop growl + auto-clear), and per-event notifications. Every
 * control writes immediately through `update` — there is no separate save step,
 * so a reload restores exactly what the trader set. A footer shows the running
 * build version, read from {@link RUNNING_RELEASE} (the same identity the release
 * watcher compares against — no second poller).
 *
 * The modal reuses the app's scrim+panel material (the SignInDialog / wizard /
 * DefaultRoutePrompt family): a dimmed, centered `role="dialog"` rendered through
 * a portal to `document.body` so it escapes any header overflow/stacking context.
 *
 * Accessible by construction: the dialog is labelled via `aria-labelledby` with
 * `aria-modal`; opening moves focus in and closing restores it to the gear; Tab
 * is focus-trapped within the panel; Escape, a scrim click, and an X button all
 * close it. Nested switches are real `role="switch"` buttons with `aria-checked`.
 */

import { useCallback, useEffect, useId, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { useSettings } from "../hooks/useSettings";
import { fmtCompact } from "../lib/format";
import { RUNNING_RELEASE, useUpdatePending } from "../data/versionManifest";
import {
  effectiveEventVolume,
  type NotificationEventType,
  type PerEventPref,
} from "../settings/settingsSchema";
import {
  SOUND_IDS,
  SOUND_LABELS,
  previewSound,
  type SoundChoice,
} from "../lib/soundKit";
import styles from "./SettingsPanel.module.css";

/** Build hashes that are not real git identities — a dev/undefined build. */
const PLACEHOLDER_HASHES: ReadonlySet<string> = new Set(["v0.0.0", "unknown", "test"]);

/** `2026-06-27T13:25:28.000Z` -> `2026-06-27 13:25 UTC` for a compact, human label. */
function humanBuildTime(iso: string): string {
  const m = /^(\d{4}-\d{2}-\d{2})T(\d{2}:\d{2})/.exec(iso);
  return m ? `${m[1]} ${m[2]} UTC` : iso;
}

/** The focusable descendants of `root`, in DOM order (for the Tab focus-trap). */
function focusableWithin(root: HTMLElement): HTMLElement[] {
  const sel =
    'a[href],button:not([disabled]),textarea,input:not([disabled]),select:not([disabled]),[tabindex]:not([tabindex="-1"])';
  return Array.from(root.querySelectorAll<HTMLElement>(sel)).filter(
    (el) => el.offsetParent !== null || el === document.activeElement,
  );
}

/** The human label + urgency flag for each configurable event row. */
const EVENT_ROWS: readonly { type: NotificationEventType; label: string }[] = [
  { type: "RfqReceived", label: "RFQ received" },
  { type: "IoiReceived", label: "IOI received" },
  { type: "ManualIntervention", label: "Needs manual pricing" },
  { type: "QuoteAccepted", label: "Quote accepted (won)" },
  { type: "QuoteRejected", label: "Quote rejected (lost)" },
  { type: "RequestLapsed", label: "Withdrawn / expired" },
  { type: "OrderReceived", label: "Order received" },
  { type: "Fill", label: "Fill" },
  { type: "FillBlock", label: "Block fill" },
];

/** The sound-picker options: every synthesized cue, plus the silent choice. */
const SOUND_OPTIONS: readonly SoundChoice[] = [...SOUND_IDS, "none"];

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

/**
 * One configurable event row: an enable toggle, a sound picker + ▶ preview, the
 * two channel checkboxes (Toast / Desktop), and a per-event volume trim. Every
 * control writes an immutable per-event patch through `onChange`.
 */
function EventRow({
  label,
  pref,
  masterVolume,
  onChange,
}: {
  label: string;
  pref: PerEventPref;
  masterVolume: number;
  onChange: (patch: Partial<PerEventPref>) => void;
}): React.ReactElement {
  const rowId = useId();
  const setChannel = (which: "toast" | "desktop", next: boolean): void =>
    onChange({ channels: { ...pref.channels, [which]: next } });
  return (
    <div className={styles.eventRow} role="group" aria-label={label}>
      <span className={styles.eventLabel}>{label}</span>
      <Switch
        checked={pref.enabled}
        onChange={(v) => onChange({ enabled: v })}
        label={`${label} enabled`}
      />
      <span className={styles.eventSound}>
        <label className={styles.srOnly} htmlFor={`${rowId}-sound`}>
          {label} sound
        </label>
        <select
          id={`${rowId}-sound`}
          className={styles.select}
          value={pref.sound}
          disabled={!pref.enabled}
          onChange={(e) => onChange({ sound: e.target.value as SoundChoice })}
        >
          {SOUND_OPTIONS.map((s) => (
            <option key={s} value={s}>
              {SOUND_LABELS[s]}
            </option>
          ))}
        </select>
        <button
          type="button"
          className={styles.previewBtn}
          aria-label={`Preview ${label} sound`}
          disabled={pref.sound === "none"}
          onClick={() =>
            previewSound(pref.sound, effectiveEventVolume(masterVolume, pref.volume))
          }
        >
          <span aria-hidden>▶</span>
        </button>
      </span>
      <span className={styles.eventChannels}>
        <label className={styles.checkLabel}>
          <input
            type="checkbox"
            checked={pref.channels.toast}
            disabled={!pref.enabled}
            onChange={(e) => setChannel("toast", e.target.checked)}
          />
          <span>Toast</span>
        </label>
        <label className={styles.checkLabel}>
          <input
            type="checkbox"
            checked={pref.channels.desktop}
            disabled={!pref.enabled}
            onChange={(e) => setChannel("desktop", e.target.checked)}
          />
          <span>Desktop</span>
        </label>
      </span>
      <span className={styles.eventVol}>
        <label className={styles.srOnly} htmlFor={`${rowId}-vol`}>
          {label} volume
        </label>
        <input
          id={`${rowId}-vol`}
          type="range"
          min={0}
          max={100}
          step={1}
          value={pref.volume}
          disabled={!pref.enabled}
          className={styles.slider}
          onChange={(e) => onChange({ volume: Number(e.target.value) })}
        />
      </span>
    </div>
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

  // On open: refresh the live permission and move focus into the panel. On close:
  // return focus to the gear (only when it was our open that is closing, tracked
  // by a ref so the mount does not steal focus).
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

  // Escape-to-close, plus a Tab focus-trap within the panel while open (matching
  // the app's other modal dialogs). The scrim owns click-outside.
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent): void => {
      if (e.key === "Escape") {
        e.stopPropagation();
        close();
        return;
      }
      if (e.key !== "Tab") return;
      const panel = panelRef.current;
      if (panel === null) return;
      const items = focusableWithin(panel);
      if (items.length === 0) {
        e.preventDefault();
        panel.focus();
        return;
      }
      const first = items[0] as HTMLElement;
      const last = items[items.length - 1] as HTMLElement;
      const active = document.activeElement;
      // The panel itself (tabIndex=-1) is the initial focus target; treat it as
      // "before first" so a forward Tab lands on the first control.
      if (e.shiftKey && (active === first || active === panel)) {
        e.preventDefault();
        last.focus();
      } else if (!e.shiftKey && active === last) {
        e.preventDefault();
        first.focus();
      }
    };
    document.addEventListener("keydown", onKey, true);
    return () => document.removeEventListener("keydown", onKey, true);
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

  const soundsOff =
    !settings.soundsEnabled || !settings.alertsEnabled || settings.masterMute;

  // Immutably patch one event's per-event preference.
  const updateEvent = useCallback(
    (type: NotificationEventType, patch: Partial<PerEventPref>) => {
      const cur = settings.perEvent[type];
      update({ perEvent: { ...settings.perEvent, [type]: { ...cur, ...patch } } });
    },
    [settings.perEvent, update],
  );

  // The "Enable desktop notifications" gesture path — request the OS grant and
  // reflect the new permission state. Guarded; never throws.
  const requestDesktopPermission = useCallback(() => {
    if (typeof window === "undefined" || !("Notification" in window)) return;
    try {
      const r = window.Notification.requestPermission();
      if (r && typeof r.then === "function") {
        r.then((p) => setPermission(p)).catch(() => {});
      } else {
        setPermission(notificationPermission());
      }
    } catch {
      /* legacy / blocked — the bell toggle remains the fallback path */
    }
  }, []);

  const hashIsPlaceholder = PLACEHOLDER_HASHES.has(RUNNING_RELEASE.hash);
  // The shared release latch (populated by the single deploy-watcher — no 2nd poll):
  // non-null means a newer build has been deployed and the app is (auto-)reloading.
  const updatePending = useUpdatePending();

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

      {open &&
        createPortal(
          <div
            className={styles.scrim}
            role="presentation"
            onMouseDown={(e) => {
              if (e.target === e.currentTarget) close();
            }}
          >
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

          <div className={styles.panelBody}>
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
              <span className={styles.rowLabel}>Mute all (do not disturb)</span>
              <Switch
                checked={settings.masterMute}
                onChange={(v) => update({ masterMute: v })}
                label="Mute all sounds"
                disabled={!settings.soundsEnabled || !settings.alertsEnabled}
              />
            </div>
            <div className={styles.row}>
              <label className={styles.rowLabel} htmlFor={`${titleId}-vol`}>
                Master volume
              </label>
              <span className={styles.sliderWrap}>
                <input
                  id={`${titleId}-vol`}
                  type="range"
                  min={0}
                  max={100}
                  step={1}
                  value={settings.masterVolume}
                  disabled={soundsOff}
                  className={styles.slider}
                  onChange={(e) => update({ masterVolume: Number(e.target.value) })}
                />
                <span className={styles.sliderValue}>{settings.masterVolume}%</span>
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

          {/* --- Per-event notifications --- */}
          <section className={styles.section} aria-labelledby={`${titleId}-events`}>
            <h3 id={`${titleId}-events`} className={styles.sectionTitle}>
              Notification events
            </h3>
            <p className={styles.hint}>
              Per event: enable, pick a sound (▶ to preview), choose channels, and
              trim its volume. A toast shows when this tab is on screen; a desktop
              banner shows only when you have tabbed away.
            </p>
            <div className={styles.row}>
              <span className={styles.rowLabel}>Desktop notifications</span>
              <button
                type="button"
                className={styles.permBtn}
                onClick={requestDesktopPermission}
                disabled={permission === "unsupported" || permission === "granted"}
              >
                {permission === "granted" ? "Enabled" : "Enable desktop notifications"}
              </button>
            </div>
            <p className={styles.hint}>{permissionHint(permission)}</p>
            <div className={styles.eventTable}>
              <div className={styles.eventHead} aria-hidden>
                <span className={styles.eventLabel}>Event</span>
                <span>On</span>
                <span>Sound</span>
                <span>Channels</span>
                <span>Vol</span>
              </div>
              {EVENT_ROWS.map((row) => (
                <EventRow
                  key={row.type}
                  label={row.label}
                  pref={settings.perEvent[row.type]}
                  masterVolume={settings.masterVolume}
                  onChange={(patch) => updateEvent(row.type, patch)}
                />
              ))}
            </div>
          </section>
          </div>

          {/* --- Version footer (plain div — NOT a <footer>, to avoid a second
              contentinfo landmark inside the app's document) --- */}
          <div className={styles.versionFoot} data-testid="settings-version">
            <span className={styles.versionLabel}>Version</span>
            <span className={styles.versionValue}>
              {hashIsPlaceholder ? (
                <span className={styles.versionHash}>
                  {humanBuildTime(RUNNING_RELEASE.buildTime)}
                </span>
              ) : (
                <>
                  <span className={styles.versionHash}>{RUNNING_RELEASE.hash}</span>
                  <span className={styles.versionTime}>
                    {humanBuildTime(RUNNING_RELEASE.buildTime)}
                  </span>
                </>
              )}
              <span
                className={styles.versionStatus}
                data-status={updatePending ? "pending" : "current"}
                data-testid="settings-update-status"
              >
                {updatePending ? "Update pending" : "Up to date"}
              </span>
            </span>
          </div>
            </div>
          </div>,
          document.body,
        )}
    </div>
  );
}
