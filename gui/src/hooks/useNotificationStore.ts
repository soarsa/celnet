/**
 * useNotificationStore — the state owner for the {@link NotificationCenter}. It
 * subscribes to the desk `streamNotifications` push channel while signed in and
 * owns everything the centre renders: the history `items`, the transient `toasts`,
 * the `unread` badge, and the dropdown `open` flag. It ALSO applies the trader's
 * {@link AppSettings} gates (master alerts, sound, size threshold, desktop growl)
 * and the two auto-clear policies (terminal-linkage + TTL sweep).
 *
 * Settings are read through a REF so the push subscription stays referentially
 * stable (it must not re-subscribe when a preference toggles). The desktop
 * ("growl") hook is instantiated ONCE here and returned, so the centre's toggle
 * and this store's `notify` share a single instance (no permission/mute desync).
 *
 * Pure decision helpers (`shouldSuppress`, `pruneOnTerminal`, `expireByTtl`) are
 * exported so the gate + auto-clear logic is unit-testable without a render.
 */

import { useCallback, useEffect, useRef, useState } from "react";
import { useApp } from "../app/AppContext";
import { useSettings } from "./useSettings";
import {
  useDesktopNotifications,
  type DesktopNotificationsApi,
} from "./useDesktopNotifications";
import { notionalMagnitude } from "../lib/notificationText";
import type { Notification, NotificationKind } from "../data/contract";
import type { AppSettings } from "../settings/settingsSchema";

/** Cap the retained notification history (newest-first ring). */
export const MAX_HISTORY = 50;
/** Cap concurrently-visible toasts (oldest dropped). */
export const MAX_TOASTS = 4;
/** Toast auto-dismiss after this long (ms). */
export const TOAST_TTL_MS = 6_000;
/** The TTL-sweep cadence (ms) — cheap: one filtered pass per tick. */
const SWEEP_INTERVAL_MS = 1_000;
/** Nanoseconds per second (age arithmetic on the ns `atNanos` field). */
const NANOS_PER_SECOND = 1_000_000_000n;
/** Nanoseconds per millisecond (to project `Date.now()` into the ns clock). */
const NANOS_PER_MILLI = 1_000_000n;

/** A live toast: a notification plus a stable key for its dismissal timer. */
export interface Toast {
  readonly key: string;
  readonly notification: Notification;
}

/** The PENDING (unresolved-request) kinds. */
function isPendingKind(kind: NotificationKind): boolean {
  return kind === "RFQ_RECEIVED" || kind === "IOI_RECEIVED";
}

/** The TERMINAL (request-resolved) kinds. */
export function isTerminalKind(kind: NotificationKind): boolean {
  return (
    kind === "QUOTE_ACCEPTED" ||
    kind === "QUOTE_REJECTED" ||
    kind === "REQUEST_WITHDRAWN" ||
    kind === "REQUEST_EXPIRED"
  );
}

/**
 * Whether an inbound notification is FULLY suppressed (no item, no toast, no
 * sound, no desktop growl). Suppressed when the master alerts switch is off, OR
 * the derived notional is below the trader's `minQty` threshold. A notification
 * whose size the heuristic cannot read (`undefined`) is NEVER suppressed by the
 * threshold — fail-open, so an unparseable headline is still shown.
 */
export function shouldSuppress(n: Notification, settings: AppSettings): boolean {
  if (!settings.alertsEnabled) return true;
  const mag = notionalMagnitude(n);
  return mag !== undefined && mag < settings.minQty;
}

/**
 * The action plan for an inbound notification — the pure decision the store
 * effect enacts. Extracted so the gate is unit-testable WITHOUT a render or spies:
 * when `suppressed`, every downstream flag is false, so a test can assert "no item,
 * no toast, no sound, no desktop growl" purely from the returned plan.
 */
export interface NotificationPlan {
  /** Fully suppressed (below min qty, or master alerts off). */
  readonly suppressed: boolean;
  /** Prepend the notification to the history list. */
  readonly addItem: boolean;
  /** Raise a transient toast. */
  readonly toast: boolean;
  /** Play the audio cue (also requires the sound preference). */
  readonly sound: boolean;
  /** Escalate to a desktop ("growl") notification (also requires the pref). */
  readonly growl: boolean;
  /** Bump the unread badge (only when the dropdown is closed). */
  readonly bumpUnread: boolean;
}

/**
 * Decide what an inbound notification should trigger, given the trader settings
 * and whether the dropdown is currently open. Pure — the store effect merely
 * enacts this.
 */
export function planNotification(
  n: Notification,
  settings: AppSettings,
  isOpen: boolean,
): NotificationPlan {
  const suppressed = shouldSuppress(n, settings);
  if (suppressed) {
    return {
      suppressed: true,
      addItem: false,
      toast: false,
      sound: false,
      growl: false,
      bumpUnread: false,
    };
  }
  // Server exception contract (commit 542e547): the server's `alert_worthy` flag
  // is the authoritative POPUP gate. An `alertWorthy:false` event still lands in
  // the centre (addItem) and still bumps the unseen count when closed, but raises
  // NO popup — no toast, no desktop growl, no sound. The kind no longer gates the
  // popup: the server now owns that decision via `alert_worthy` (so a manual-
  // intervention kind 7, neither a *_RECEIVED nor a terminal kind, still pops when
  // alert-worthy; an auto-priced event with alert_worthy:false stays quiet).
  return {
    suppressed: false,
    addItem: true,
    toast: n.alertWorthy,
    sound: n.alertWorthy && settings.soundsEnabled,
    growl: n.alertWorthy && settings.growlEnabled,
    bumpUnread: !isOpen,
  };
}

/**
 * Auto-clear (a) terminal-linkage: when a TERMINAL notification carrying a
 * `requestId` arrives, drop any still-present PENDING notifications for the SAME
 * `requestId` — the underlying request is now resolved, so its "awaiting price"
 * entry is stale. A no-op for non-terminal inbounds / missing `requestId`.
 */
export function pruneOnTerminal(
  items: readonly Notification[],
  incoming: Notification,
): Notification[] {
  if (!isTerminalKind(incoming.kind) || incoming.requestId === undefined) {
    return [...items];
  }
  return items.filter(
    (n) => !(isPendingKind(n.kind) && n.requestId === incoming.requestId),
  );
}

/**
 * Auto-clear (b) TTL sweep: drop any notification whose age
 * `(nowNanos - atNanos)` exceeds `ttlSeconds`. A non-positive TTL disables the
 * sweep (returns the list unchanged) so "0" never nukes the list instantly.
 */
export function expireByTtl(
  items: readonly Notification[],
  nowNanos: bigint,
  ttlSeconds: number,
): Notification[] {
  if (ttlSeconds <= 0) return [...items];
  const ttlNanos = BigInt(Math.floor(ttlSeconds)) * NANOS_PER_SECOND;
  return items.filter((n) => nowNanos - n.atNanos <= ttlNanos);
}

// --- audio cue (WebAudio, lazy, guarded) ------------------------------------

/** A lazily-created shared AudioContext (a single node graph is cheap to reuse). */
let sharedAudioCtx: AudioContext | null = null;

/** The AudioContext ctor across browsers (`webkit`-prefixed on old Safari). */
function audioCtxCtor(): (new () => AudioContext) | null {
  if (typeof window === "undefined") return null;
  const w = window as unknown as {
    AudioContext?: new () => AudioContext;
    webkitAudioContext?: new () => AudioContext;
  };
  return w.AudioContext ?? w.webkitAudioContext ?? null;
}

/** Obtain (once) the shared AudioContext, or null when unsupported. Never throws. */
function getAudioCtx(): AudioContext | null {
  try {
    if (sharedAudioCtx) return sharedAudioCtx;
    const Ctor = audioCtxCtor();
    if (!Ctor) return null;
    sharedAudioCtx = new Ctor();
    return sharedAudioCtx;
  } catch {
    return null;
  }
}

/**
 * Play a short, gentle cue. `volume` is 0–100 (scaled into a low gain ceiling so
 * the beep never blares). Terminal events use a slightly lower pitch so accepted/
 * rejected reads different from a new request. Fully guarded — an unsupported or
 * suspended context degrades to a silent no-op and NEVER throws.
 */
export function playCue(volume: number, terminal: boolean): void {
  try {
    const vol = Math.max(0, Math.min(100, volume)) / 100;
    if (vol <= 0) return;
    const ctx = getAudioCtx();
    if (!ctx) return;
    if (ctx.state === "suspended") void ctx.resume().catch(() => {});
    const osc = ctx.createOscillator();
    const gain = ctx.createGain();
    osc.type = "sine";
    osc.frequency.value = terminal ? 523 : 740;
    const now = ctx.currentTime;
    const peak = Math.max(0.0002, vol * 0.14);
    gain.gain.setValueAtTime(0.0001, now);
    gain.gain.exponentialRampToValueAtTime(peak, now + 0.012);
    gain.gain.exponentialRampToValueAtTime(0.0001, now + 0.18);
    osc.connect(gain).connect(ctx.destination);
    osc.start(now);
    osc.stop(now + 0.2);
  } catch {
    /* audio unsupported / blocked — never throw into the render tree */
  }
}

// --- the store hook ---------------------------------------------------------

/** The public surface the {@link NotificationCenter} renders from. */
export interface NotificationStore {
  readonly items: Notification[];
  readonly toasts: Toast[];
  readonly unread: number;
  readonly open: boolean;
  toggleOpen: () => void;
  /** Remove a single notification from the history (per-item ×). */
  dismiss: (notificationId: string) => void;
  /** Empty the history and reset the unread badge. */
  clearAll: () => void;
  /** Route to the Quoting desk for a notification and close the dropdown. */
  openRequest: (n: Notification) => void;
  /** The shared desktop-notifications instance (toggle state + permission). */
  readonly desktop: DesktopNotificationsApi;
}

export function useNotificationStore(): NotificationStore {
  const app = useApp();
  const { settings } = useSettings();
  const signedIn = app.auth.user !== null;

  // Native OS ("growl") escalation, instantiated ONCE here. Clicking the OS
  // notification focuses the window and routes to the Quoting desk.
  const desktop = useDesktopNotifications(() => app.setWorkspace("quoting"));
  const { notify: notifyDesktop } = desktop;

  const [items, setItems] = useState<Notification[]>([]);
  const [toasts, setToasts] = useState<Toast[]>([]);
  const [unread, setUnread] = useState(0);
  const [open, setOpen] = useState(false);

  // Mirror `open` + `settings` into refs so the push handler reads the latest
  // WITHOUT being re-created (which would tear down and re-open the stream).
  const openRef = useRef(open);
  openRef.current = open;
  const settingsRef = useRef(settings);
  settingsRef.current = settings;

  const dismissToast = useCallback((key: string) => {
    setToasts((ts) => ts.filter((t) => t.key !== key));
  }, []);

  // Open the push stream while signed in. The handler applies the settings gate,
  // the terminal-linkage prune, then (if not suppressed) prepends to history,
  // bumps unread, raises a toast, plays the cue, and escalates to the OS.
  useEffect(() => {
    if (!signedIn) {
      setItems([]);
      setToasts([]);
      setUnread(0);
      return;
    }
    const onNotification = (n: Notification): void => {
      const s = settingsRef.current;
      const plan = planNotification(n, s, openRef.current);
      // Terminal-linkage prune runs regardless of suppression (lifecycle truth),
      // then the incoming is prepended only when it survives the gate.
      setItems((prev) => {
        const pruned = pruneOnTerminal(prev, n);
        if (!plan.addItem) return pruned;
        return [n, ...pruned].slice(0, MAX_HISTORY);
      });
      if (plan.suppressed) return;
      if (plan.bumpUnread) setUnread((u) => u + 1);
      if (plan.toast) {
        const toast: Toast = {
          key: `${n.notificationId}-${n.atNanos.toString()}`,
          notification: n,
        };
        setToasts((ts) => [...ts, toast].slice(-MAX_TOASTS));
        window.setTimeout(() => dismissToast(toast.key), TOAST_TTL_MS);
      }
      if (plan.sound) playCue(s.volume, isTerminalKind(n.kind));
      // Desktop growl: gated by the settings master AND the hook's own
      // tab-hidden + permission + mute checks (so we never double-notify).
      if (plan.growl) notifyDesktop(n);
    };
    const dispose = app.transport.streamNotifications(undefined, onNotification);
    return dispose;
  }, [app.transport, signedIn, dismissToast, notifyDesktop]);

  // TTL sweep: a single stable timer whose body reads the live settings via ref,
  // so toggling `autoClearCompleted` off simply makes each tick a no-op (no
  // effect churn). Removes anything older than the configured TTL.
  useEffect(() => {
    if (!signedIn) return;
    const id = window.setInterval(() => {
      const s = settingsRef.current;
      if (!s.autoClearCompleted) return;
      const nowNanos = BigInt(Date.now()) * NANOS_PER_MILLI;
      setItems((prev) => {
        const next = expireByTtl(prev, nowNanos, s.autoClearTtlSeconds);
        return next.length === prev.length ? prev : next;
      });
    }, SWEEP_INTERVAL_MS);
    return () => window.clearInterval(id);
  }, [signedIn]);

  const toggleOpen = useCallback(() => {
    setOpen((o) => {
      const next = !o;
      if (next) setUnread(0);
      return next;
    });
  }, []);

  const dismiss = useCallback((notificationId: string) => {
    setItems((prev) => prev.filter((n) => n.notificationId !== notificationId));
  }, []);

  const clearAll = useCallback(() => {
    setItems([]);
    setUnread(0);
  }, []);

  const openRequest = useCallback(
    (n: Notification) => {
      app.setWorkspace("quoting");
      setOpen(false);
      setUnread(0);
      void n;
    },
    [app],
  );

  return {
    items,
    toasts,
    unread,
    open,
    toggleOpen,
    dismiss,
    clearAll,
    openRequest,
    desktop,
  };
}
