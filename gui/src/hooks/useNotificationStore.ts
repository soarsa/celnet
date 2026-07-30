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
  tabIsAway,
  type DesktopNotificationsApi,
} from "./useDesktopNotifications";
import { notionalMagnitude } from "../lib/notificationText";
import { playSound, type SoundChoice, type SoundId } from "../lib/soundKit";
import type { Notification, NotificationKind } from "../data/contract";
import {
  eventTypeForKind,
  effectiveEventVolume,
  DEFAULT_PER_EVENT,
  type AppSettings,
} from "../settings/settingsSchema";

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
 * when `suppressed`, every downstream flag is false / silent, so a test can assert
 * "no item, no toast, no desktop, no sound" purely from the returned plan.
 */
export interface NotificationPlan {
  /** Fully suppressed (below min qty, or master alerts off). */
  readonly suppressed: boolean;
  /** Prepend the notification to the history list. */
  readonly addItem: boolean;
  /** Raise an in-app "growl" toast (focus-aware: only when the tab is on screen). */
  readonly toast: boolean;
  /** Escalate to an OS desktop banner (focus-aware: only when the tab is away). */
  readonly desktop: boolean;
  /** The cue to play (`"none"` ⇒ silent) — the per-event configured sound. */
  readonly sound: SoundChoice;
  /** The effective cue volume (0–100), master × per-event trim. */
  readonly volume: number;
  /** Bump the unread badge (only when the dropdown is closed). */
  readonly bumpUnread: boolean;
}

/**
 * Decide what an inbound notification should trigger, given the trader settings,
 * whether the dropdown is open, and whether the tab is currently AWAY (hidden /
 * unfocused). Pure — the store effect merely enacts this (the streak override is
 * applied there, since it is stateful).
 *
 * The gate is layered:
 *   1. `shouldSuppress` — master alerts off / below min-qty ⇒ nothing at all.
 *   2. The server's `alert_worthy` flag (commit 542e547) AND the per-event
 *      `enabled` toggle form the POPUP FLOOR — a quiet event or a trader-disabled
 *      event still lands in the centre (addItem) + bumps unread, but raises no
 *      toast/desktop/sound. The server can never be over-ridden UP: a client
 *      config can only silence, never force, a popup the server marked quiet.
 *   3. The FOCUS-AWARE rule (§5.3) picks the channel: an in-app toast when the tab
 *      is on screen, an OS banner ONLY when it is away — never both. Per-event
 *      `channels` narrow this further (toast-only / desktop-only / both / off).
 *   4. The sound is the event's configured cue, at master × per-event volume,
 *      gated by the sound master + do-not-disturb `masterMute`.
 */
export function planNotification(
  n: Notification,
  settings: AppSettings,
  isOpen: boolean,
  isAway: boolean,
): NotificationPlan {
  const suppressed = shouldSuppress(n, settings);
  if (suppressed) {
    return {
      suppressed: true,
      addItem: false,
      toast: false,
      desktop: false,
      sound: "none",
      volume: 0,
      bumpUnread: false,
    };
  }
  const pref = settings.perEvent[eventTypeForKind(n.kind)] ?? DEFAULT_PER_EVENT.RfqReceived;
  const base = n.alertWorthy && pref.enabled;
  const toast = base && pref.channels.toast && !isAway;
  const desktop = base && pref.channels.desktop && isAway && settings.growlEnabled;
  const soundOn =
    base && settings.soundsEnabled && !settings.masterMute && pref.sound !== "none";
  return {
    suppressed: false,
    addItem: true,
    toast,
    desktop,
    sound: soundOn ? pref.sound : "none",
    volume: soundOn ? effectiveEventVolume(settings.masterVolume, pref.volume) : 0,
    bumpUnread: !isOpen,
  };
}

/** The rolling state for the fill-streak escalation. */
export interface StreakState {
  /** Consecutive fill count within the streak window (1 = a fresh streak). */
  readonly count: number;
  /** When the last fill landed (ms epoch), for the window comparison. */
  readonly lastMs: number;
}

/** A pristine (no-streak) starting state. */
export const EMPTY_STREAK: StreakState = { count: 0, lastMs: 0 };

/**
 * Advance the streak on a fill: increment when the previous fill is within
 * `windowMs`, else restart the count at 1. Pure — the store owns the ref.
 */
export function advanceStreak(prev: StreakState, nowMs: number, windowMs: number): StreakState {
  const within = prev.count > 0 && nowMs - prev.lastMs <= windowMs;
  return { count: within ? prev.count + 1 : 1, lastMs: nowMs };
}

/**
 * Whether a kind counts toward the fill-streak. The phase-5 `FILL` kind (an own
 * execution booked — the FIX-venue firm-order lift) is the primary own-fill; the
 * booked-deal `QUOTE_ACCEPTED` (a lifted RFQ quote) is also an own fill and stays
 * included so RFQ lifts keep their fill cue/streak. (`FillBlock` is a client-side
 * size derivation over a `FILL`, not a distinct wire kind.)
 */
export function isFillKind(kind: NotificationKind): boolean {
  return kind === "FILL" || kind === "QUOTE_ACCEPTED";
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

// --- audio cue --------------------------------------------------------------

/**
 * Play a short, gentle cue — a thin back-compat shim over the {@link playSound}
 * sound kit: a terminal event uses the neutral `lapsed` cue, a pending one the
 * `rfq-work` blip. `volume` is 0–100. Fully guarded — never throws. (Live routing
 * now goes through `planNotification` → the per-event configured cue; this remains
 * for callers that only know "pending vs terminal".)
 */
export function playCue(volume: number, terminal: boolean): void {
  playSound(terminal ? "lapsed" : "rfq-work", volume);
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
  // The fill-streak counter, kept in a ref so the stable push handler mutates it
  // without re-subscribing. Reset whenever the stream tears down (sign-out).
  const streakRef = useRef<StreakState>(EMPTY_STREAK);

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
      streakRef.current = EMPTY_STREAK;
      return;
    }
    const onNotification = (n: Notification): void => {
      const s = settingsRef.current;
      // Focus-aware channel routing hinges on whether the tab is on screen NOW.
      const away = tabIsAway();
      const plan = planNotification(n, s, openRef.current, away);
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
      if (plan.sound !== "none") {
        let cue: SoundId = plan.sound;
        let streak = 0;
        // A rapid run of fills coalesces into the escalating fill-streak ladder
        // (§4.3) — its height encodes the consecutive-fill count.
        if (isFillKind(n.kind)) {
          const next = advanceStreak(streakRef.current, Date.now(), s.streakWindowMs);
          streakRef.current = next;
          if (next.count >= 2) {
            cue = "fill-streak";
            streak = next.count;
          }
        }
        playSound(cue, plan.volume, streak);
      }
      // Desktop banner: the plan already gated it on the focus-aware away rule +
      // the growl master; the hook adds its own permission/mute checks.
      if (plan.desktop) notifyDesktop(n);
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
