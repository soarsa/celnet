/**
 * useDesktopNotifications — native OS ("growl") desktop notifications for the desk
 * notification stream, layered ON TOP of the in-app {@link NotificationCenter}.
 *
 * The rule: a desk event (new RFQ/IOI, a quote sent/auto-quoted, a quote
 * accepted/rejected) raises a NATIVE `Notification` ONLY when the browser tab is
 * NOT visible or NOT focused — when the tab is on screen the in-app centre already
 * shows the toast + bell, so an OS notification would double-notify. When the tab
 * is hidden/backgrounded, the trader would otherwise miss the event entirely, so
 * the OS surface is the right escalation.
 *
 * Permission is requested ONCE — either by a user gesture on the toggle this hook
 * powers, or a single guarded first-mount request — and the outcome
 * (`granted`/`denied`/`default`) plus the browser's support state are surfaced so
 * the toggle can reflect reality. A user PREFERENCE (independent of the browser
 * grant) is persisted to `localStorage`, so a trader can mute desktop alerts
 * without revoking the OS grant. Every Web-Notifications call is guarded — an
 * unsupported browser, a blocked origin, or a throwing constructor degrades to a
 * silent no-op and NEVER throws into the render tree.
 */

import { useCallback, useEffect, useRef, useState } from "react";
import type { Notification as DeskNotification } from "../data/contract";
import { manualInterventionText } from "../lib/notificationText";

/**
 * The body text for the native OS notification. For a
 * `MANUAL_INTERVENTION_REQUIRED` event carrying a `reason`, render the
 * trader-facing reason (via {@link manualInterventionText}) combined with any
 * `detail`; otherwise fall back to the plain `detail` string.
 */
function desktopBody(n: DeskNotification): string {
  if (n.kind === "MANUAL_INTERVENTION_REQUIRED" && n.reason !== undefined) {
    const reasonText = manualInterventionText(n.reason);
    return n.detail ? `${n.detail} — ${reasonText}` : reasonText;
  }
  return n.detail ?? "";
}

/** The user preference (mute/unmute), independent of the browser grant. */
const PREF_KEY = "celnet.desktopNotifications.enabled";
/** A one-shot guard so the first-mount auto-request prompts at most once, ever. */
const ASKED_KEY = "celnet.desktopNotifications.asked";

/** The browser permission, widened with an `unsupported` sentinel for old browsers. */
export type DesktopPermission = NotificationPermission | "unsupported";

/** The public surface the {@link NotificationCenter} toggle + stream handler use. */
export interface DesktopNotificationsApi {
  /** Whether the Web Notifications API exists in this browser. */
  readonly supported: boolean;
  /** The current browser permission (or `unsupported`). */
  readonly permission: DesktopPermission;
  /** The user preference (mute state) — independent of the browser grant. */
  readonly preferenceEnabled: boolean;
  /** Effective on: supported AND granted AND the user hasn't muted. */
  readonly enabled: boolean;
  /**
   * Toggle desktop alerts. When permission is still `default` this requests it
   * (the user-gesture path); when already `granted` it flips the mute preference;
   * when `denied`/`unsupported` it is a no-op (the browser owns that decision).
   */
  toggle: () => void;
  /**
   * Best-effort raise an OS notification for a desk event. A no-op unless
   * {@link enabled} AND the tab is hidden or unfocused (so we never double-notify
   * over the in-app centre). Never throws.
   */
  notify: (n: DeskNotification) => void;
}

/** Whether the Web Notifications API is available in this environment. */
function isSupported(): boolean {
  return typeof window !== "undefined" && "Notification" in window;
}

/** Read a persisted boolean flag; missing/unreadable ⇒ `fallback`. */
function readFlag(key: string, fallback: boolean): boolean {
  try {
    const raw = window.localStorage.getItem(key);
    return raw === null ? fallback : raw === "1";
  } catch {
    return fallback;
  }
}

/** Persist a boolean flag; failures (private mode, quota) are swallowed. */
function writeFlag(key: string, value: boolean): void {
  try {
    window.localStorage.setItem(key, value ? "1" : "0");
  } catch {
    /* storage unavailable — the preference is best-effort, never fatal */
  }
}

/** Whether the tab is currently hidden or unfocused (the escalation condition). */
export function tabIsAway(): boolean {
  if (typeof document === "undefined") return false;
  return document.visibilityState === "hidden" || !document.hasFocus();
}

/**
 * @param onActivate best-effort navigation invoked when the user clicks the OS
 *        notification (after `window.focus()`), e.g. route to the Quoting desk.
 */
export function useDesktopNotifications(onActivate?: () => void): DesktopNotificationsApi {
  const supported = isSupported();

  const [permission, setPermission] = useState<DesktopPermission>(() =>
    supported ? window.Notification.permission : "unsupported",
  );
  const [preferenceEnabled, setPreferenceEnabled] = useState<boolean>(() =>
    readFlag(PREF_KEY, true),
  );

  // Keep the activation callback in a ref so `notify` stays referentially stable
  // (the stream subscription that calls it must not re-subscribe on every render).
  const onActivateRef = useRef(onActivate);
  onActivateRef.current = onActivate;

  const enabled = supported && permission === "granted" && preferenceEnabled;

  // Mirror `enabled` into a ref for the same stable-`notify` reason.
  const enabledRef = useRef(enabled);
  enabledRef.current = enabled;

  /** Apply a permission outcome and, on first grant, unmute the preference. */
  const applyPermission = useCallback((result: NotificationPermission) => {
    setPermission(result);
    if (result === "granted") {
      setPreferenceEnabled(true);
      writeFlag(PREF_KEY, true);
    }
  }, []);

  /** Ask the browser for permission, tolerating the legacy callback-only form. */
  const requestPermission = useCallback(() => {
    if (!isSupported()) return;
    writeFlag(ASKED_KEY, true);
    try {
      // Modern browsers return a promise; legacy Safari only takes a callback and
      // returns undefined — support both without throwing.
      const result = window.Notification.requestPermission(applyPermission);
      if (result && typeof result.then === "function") {
        result.then(applyPermission).catch(() => {
          /* user dismissed / browser refused — leave state as-is */
        });
      }
    } catch {
      /* requestPermission threw (very old browsers) — degrade silently */
    }
  }, [applyPermission]);

  // First-mount, one-time auto-request: only when supported, still undecided, the
  // user hasn't muted, and we've never asked before. The toggle is the primary
  // (gesture) path; this is the "or on first mount" path, guarded so it never nags.
  useEffect(() => {
    if (!supported) return;
    if (permission !== "default") return;
    if (!preferenceEnabled) return;
    if (readFlag(ASKED_KEY, false)) return;
    requestPermission();
    // Intentionally mount-only: we want exactly one auto-prompt for a session's
    // first sight of the centre, not a re-prompt on every dependency change.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const toggle = useCallback(() => {
    if (!isSupported()) return;
    const current = window.Notification.permission;
    if (current === "denied") return; // browser-blocked; nothing we can do
    if (current === "default") {
      requestPermission();
      return;
    }
    // Already granted — flip the mute preference and persist it.
    setPreferenceEnabled((prev) => {
      const next = !prev;
      writeFlag(PREF_KEY, next);
      return next;
    });
  }, [requestPermission]);

  const notify = useCallback((n: DeskNotification) => {
    if (!enabledRef.current) return;
    if (!tabIsAway()) return; // tab on screen ⇒ in-app centre already shows it
    try {
      // `renotify` is a valid Web Notifications option (suppress re-alerting when a
      // notification with the same `tag` replaces an earlier one) but is absent from
      // TS's lib.dom `NotificationOptions`, so we widen the literal's type.
      const options: NotificationOptions & { renotify?: boolean } = {
        body: desktopBody(n),
        tag: n.notificationId,
        renotify: false,
      };
      const osNote = new window.Notification(n.headline, options);
      osNote.onclick = () => {
        try {
          window.focus();
          onActivateRef.current?.();
        } finally {
          osNote.close();
        }
      };
    } catch {
      /* constructor blocked (e.g. permission raced to denied) — silent no-op */
    }
  }, []);

  return { supported, permission, preferenceEnabled, enabled, toggle, notify };
}
