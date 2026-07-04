/**
 * NotificationCenter — the global, signed-in-only desk notification surface. It
 * opens the dedicated `streamNotifications` push channel once a user is signed in
 * and renders TWO affordances from the same feed:
 *   1. transient TOASTS for new RFQ/IOI-requires-pricing events (auto-dismissed),
 *      announced via an `aria-live` region; and
 *   2. a persistent BELL + unread badge whose dropdown lists recent events;
 *      clicking an item routes to the Quoting workspace.
 *
 * One contract, two transports: the feed is the `CelnetTransport.streamNotifications`
 * seam, so the SAME notifications drive the centre through the deterministic in-app
 * source (local emitter) and the live `NotificationService` edge (a re-opening WS
 * push stream). Styled with the banner family (UpdateBanner / ArbBanner).
 */

import { useCallback, useEffect, useRef, useState } from "react";
import { useApp } from "../app/AppContext";
import { useDesktopNotifications } from "../hooks/useDesktopNotifications";
import { fmtClock } from "../lib/format";
import type { Notification, NotificationKind } from "../data/contract";
import styles from "./NotificationCenter.module.css";

/** Cap the retained notification history (newest-first ring). */
const MAX_HISTORY = 50;
/** Cap concurrently-visible toasts (oldest dropped). */
const MAX_TOASTS = 4;
/** Toast auto-dismiss after this long (ms). */
const TOAST_TTL_MS = 6_000;

/** A live toast: a notification plus its own dismissal timer id. */
interface Toast {
  readonly key: string;
  readonly notification: Notification;
}

/** The kinds that raise a transient toast (a new inbound request needing a price). */
function isToastKind(kind: NotificationKind): boolean {
  return kind === "RFQ_RECEIVED" || kind === "IOI_RECEIVED";
}

/** The badge accent class for a notification kind. */
function kindClass(kind: NotificationKind): string {
  switch (kind) {
    case "RFQ_RECEIVED":
    case "IOI_RECEIVED":
      return styles.dotNew ?? "";
    case "QUOTE_ACCEPTED":
      return styles.dotAccepted ?? "";
    case "QUOTE_REJECTED":
      return styles.dotRejected ?? "";
    default:
      return styles.dotInert ?? "";
  }
}

/** The a11y label + tooltip for the desktop-notifications toggle, per its state. */
function desktopToggleTitle(d: {
  supported: boolean;
  permission: string;
  enabled: boolean;
}): string {
  if (!d.supported) return "Desktop notifications not supported in this browser";
  if (d.permission === "denied")
    return "Desktop notifications blocked — allow them in your browser settings";
  if (d.permission === "default") return "Enable desktop notifications";
  return d.enabled
    ? "Desktop notifications on — click to mute"
    : "Desktop notifications off — click to enable";
}

export function NotificationCenter(): React.ReactElement | null {
  const app = useApp();
  const signedIn = app.auth.user !== null;

  // Native OS ("growl") escalation: raise a desktop notification for a desk event
  // only when the tab is hidden/unfocused (the hook enforces that). Clicking the
  // OS notification focuses the window and routes to the Quoting desk.
  const desktop = useDesktopNotifications(() => app.setWorkspace("quoting"));
  const { notify: notifyDesktop } = desktop;

  const [items, setItems] = useState<Notification[]>([]);
  const [toasts, setToasts] = useState<Toast[]>([]);
  const [unread, setUnread] = useState(0);
  const [open, setOpen] = useState(false);

  // A ref mirror of `open` so the push handler knows whether to bump the unread
  // count WITHOUT being re-created (which would re-subscribe the stream).
  const openRef = useRef(open);
  openRef.current = open;

  const dismissToast = useCallback((key: string) => {
    setToasts((ts) => ts.filter((t) => t.key !== key));
  }, []);

  // Open the push stream while signed in. The handler prepends to history, bumps
  // the unread count (unless the dropdown is open), and raises a toast for a new
  // inbound request. The returned disposer unsubscribes on sign-out / unmount.
  useEffect(() => {
    if (!signedIn) {
      setItems([]);
      setToasts([]);
      setUnread(0);
      return;
    }
    const onNotification = (n: Notification): void => {
      setItems((prev) => [n, ...prev].slice(0, MAX_HISTORY));
      if (!openRef.current) setUnread((u) => u + 1);
      if (isToastKind(n.kind)) {
        const toast: Toast = { key: `${n.notificationId}-${n.atNanos.toString()}`, notification: n };
        setToasts((ts) => [...ts, toast].slice(-MAX_TOASTS));
        window.setTimeout(() => dismissToast(toast.key), TOAST_TTL_MS);
      }
      // Escalate to a native OS notification when the tab is backgrounded (the
      // hook no-ops when the tab is on screen, so we never double-notify).
      notifyDesktop(n);
    };
    const dispose = app.transport.streamNotifications(undefined, onNotification);
    return dispose;
  }, [app.transport, signedIn, dismissToast, notifyDesktop]);

  const toggleOpen = useCallback(() => {
    setOpen((o) => {
      const next = !o;
      if (next) setUnread(0);
      return next;
    });
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

  if (!signedIn) return null;

  const desktopTitle = desktopToggleTitle(desktop);
  const desktopDisabled = !desktop.supported || desktop.permission === "denied";

  return (
    <div className={styles.root}>
      <button
        type="button"
        className={styles.toggle}
        aria-pressed={desktop.enabled}
        aria-label={desktopTitle}
        title={desktopTitle}
        disabled={desktopDisabled}
        onClick={desktop.toggle}
      >
        <span className={styles.toggleGlyph} aria-hidden>
          {desktop.enabled ? "◉" : "◎"}
        </span>
      </button>
      <button
        type="button"
        className={styles.bell}
        aria-label={
          unread > 0 ? `Notifications, ${unread} unread` : "Notifications, none unread"
        }
        aria-expanded={open}
        aria-haspopup="menu"
        onClick={toggleOpen}
      >
        <span className={styles.bellGlyph} aria-hidden>
          ◔
        </span>
        {unread > 0 && (
          <span className={styles.badge} aria-hidden>
            {unread > 9 ? "9+" : unread}
          </span>
        )}
      </button>

      {open && (
        <div className={styles.dropdown} role="menu" aria-label="Recent notifications">
          <header className={styles.dropHead}>
            <span>Notifications</span>
            <span className={styles.dropCount}>{items.length}</span>
          </header>
          {items.length === 0 ? (
            <p className={styles.dropEmpty}>No notifications yet.</p>
          ) : (
            <ul className={styles.dropList}>
              {items.map((n) => (
                <li key={`${n.notificationId}-${n.atNanos.toString()}`}>
                  <button
                    type="button"
                    role="menuitem"
                    className={styles.dropItem}
                    onClick={() => openRequest(n)}
                  >
                    <span className={`${styles.dot} ${kindClass(n.kind)}`} aria-hidden />
                    <span className={styles.dropBody}>
                      <span className={styles.dropHeadline}>{n.headline}</span>
                      {n.detail && <span className={styles.dropDetail}>{n.detail}</span>}
                    </span>
                    <span className={styles.dropTime}>{fmtClock(n.atNanos)}</span>
                  </button>
                </li>
              ))}
            </ul>
          )}
        </div>
      )}

      {/* Transient toasts — announced politely, never stealing focus. */}
      <div className={styles.toasts} role="status" aria-live="polite">
        {toasts.map((t) => (
          <button
            type="button"
            key={t.key}
            className={styles.toast}
            onClick={() => {
              openRequest(t.notification);
              dismissToast(t.key);
            }}
          >
            <span className={`${styles.dot} ${kindClass(t.notification.kind)}`} aria-hidden />
            <span className={styles.toastBody}>
              <span className={styles.toastHeadline}>{t.notification.headline}</span>
              {t.notification.detail && (
                <span className={styles.toastDetail}>{t.notification.detail}</span>
              )}
            </span>
            <span className={styles.toastAction}>Price →</span>
          </button>
        ))}
      </div>
    </div>
  );
}
