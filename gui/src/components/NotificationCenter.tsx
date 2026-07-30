/**
 * NotificationCenter — the global, signed-in-only desk notification surface. It
 * opens the dedicated `streamNotifications` push channel once a user is signed in
 * and renders TWO affordances from the same feed:
 *   1. transient TOASTS for new RFQ/IOI-requires-pricing events (auto-dismissed),
 *      announced via an `aria-live` region; and
 *   2. a persistent BELL + unread badge whose dropdown lists recent events, with a
 *      Clear-all control and a per-item dismiss ×; clicking an item routes to the
 *      Quoting workspace.
 *
 * All state (items/toasts/unread/open) + the settings gates (alerts, sound, size
 * threshold, desktop growl) + the auto-clear policies live in
 * {@link useNotificationStore}; this component is the presentation layer. Human
 * headlines/details are rendered through {@link compactNotionals} so a raw
 * "10000000 notional" reads as "10m notional". Styled with the banner family.
 */

import { useApp } from "../app/AppContext";
import {
  useNotificationStore,
  isTerminalKind,
} from "../hooks/useNotificationStore";
import { fmtClock } from "../lib/format";
import { compactNotionals, manualInterventionText } from "../lib/notificationText";
import type { Notification, NotificationKind } from "../data/contract";
import styles from "./NotificationCenter.module.css";

/**
 * The body detail line for a notification: for a `MANUAL_INTERVENTION_REQUIRED`
 * event carrying a `reason`, the trader-facing reason label; otherwise the
 * compacted `detail`. Returns null when there is nothing to render.
 */
function detailLine(n: Notification): string | null {
  if (n.kind === "MANUAL_INTERVENTION_REQUIRED" && n.reason !== undefined) {
    const reasonText = manualInterventionText(n.reason);
    return n.detail ? `${compactNotionals(n.detail)} — ${reasonText}` : reasonText;
  }
  return n.detail ? compactNotionals(n.detail) : null;
}

/**
 * Whether a kind is URGENT — it demands the trader's attention (manual pricing
 * needed, or a quote declined). Urgent toasts are announced ASSERTIVELY
 * (`role="alert"`), routine ones POLITELY (`role="status"`).
 */
function isUrgentKind(kind: NotificationKind): boolean {
  return kind === "MANUAL_INTERVENTION_REQUIRED" || kind === "QUOTE_REJECTED";
}

/** The badge accent class for a notification kind. */
function kindClass(kind: NotificationKind): string {
  switch (kind) {
    case "RFQ_RECEIVED":
    case "IOI_RECEIVED":
    case "ORDER_RECEIVED":
      return styles.dotNew ?? "";
    case "QUOTE_ACCEPTED":
    case "FILL":
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

  const store = useNotificationStore();
  const { items, toasts, unread, open, toggleOpen, dismiss, clearAll, openRequest, desktop } =
    store;

  if (!signedIn) return null;

  const desktopTitle = desktopToggleTitle(desktop);
  const desktopDisabled = !desktop.supported || desktop.permission === "denied";
  // Escalate the live-region politeness when any visible toast is urgent, so a
  // screen reader interrupts for a "needs you" / declined-quote event.
  const hasUrgentToast = toasts.some((t) => isUrgentKind(t.notification.kind));

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
        aria-haspopup="true"
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
        <div className={styles.dropdown} role="group" aria-label="Recent notifications">
          <div className={styles.dropHead}>
            <span>Notifications</span>
            <span className={styles.dropHeadRight}>
              <span className={styles.dropCount}>{items.length}</span>
              {items.length > 0 && (
                <button
                  type="button"
                  className={styles.clearAll}
                  aria-label="Clear all notifications"
                  onClick={clearAll}
                >
                  Clear all
                </button>
              )}
            </span>
          </div>
          {items.length === 0 ? (
            <p className={styles.dropEmpty}>No notifications yet.</p>
          ) : (
            <ul className={styles.dropList}>
              {items.map((n) => (
                <li key={`${n.notificationId}-${n.atNanos.toString()}`} className={styles.dropRow}>
                  <button
                    type="button"
                    className={styles.dropItem}
                    onClick={() => openRequest(n)}
                  >
                    <span className={`${styles.dot} ${kindClass(n.kind)}`} aria-hidden />
                    <span className={styles.dropBody}>
                      <span className={styles.dropHeadline}>{compactNotionals(n.headline)}</span>
                      {detailLine(n) && (
                        <span className={styles.dropDetail}>{detailLine(n)}</span>
                      )}
                    </span>
                    <span className={styles.dropTime}>{fmtClock(n.atNanos)}</span>
                  </button>
                  <button
                    type="button"
                    className={styles.dismiss}
                    aria-label="Dismiss notification"
                    onClick={(e) => {
                      e.stopPropagation();
                      dismiss(n.notificationId);
                    }}
                  >
                    <span aria-hidden>×</span>
                  </button>
                </li>
              ))}
            </ul>
          )}
        </div>
      )}

      {/* Transient toasts — announced via a live region (assertive when urgent),
          never stealing focus. */}
      <div
        className={styles.toasts}
        role={hasUrgentToast ? "alert" : "status"}
        aria-live={hasUrgentToast ? "assertive" : "polite"}
      >
        {toasts.map((t) => (
          <button
            type="button"
            key={t.key}
            className={styles.toast}
            onClick={() => openRequest(t.notification)}
          >
            <span className={`${styles.dot} ${kindClass(t.notification.kind)}`} aria-hidden />
            <span className={styles.toastBody}>
              <span className={styles.toastHeadline}>
                {compactNotionals(t.notification.headline)}
              </span>
              {detailLine(t.notification) && (
                <span className={styles.toastDetail}>
                  {detailLine(t.notification)}
                </span>
              )}
            </span>
            <span className={styles.toastAction}>
              {isTerminalKind(t.notification.kind) ? "View →" : "Price →"}
            </span>
          </button>
        ))}
      </div>
    </div>
  );
}
