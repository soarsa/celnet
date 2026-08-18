/**
 * useDesktopNotifications — native ("growl") escalation for the desk notification
 * stream. These tests cover the graceful degradation contract (unsupported /
 * denied browsers are silent no-ops that never throw) and the server-exception
 * contract body rendering (a MANUAL_INTERVENTION_REQUIRED event's OS-notification
 * body carries the reason label via `manualInterventionText`).
 *
 * The Web Notifications API is mocked: jsdom does NOT implement `window.Notification`
 * (verified — `"Notification" in window` is false by default), so the base env IS
 * the "unsupported" case, and a `FakeNotification` is stubbed in for the supported
 * cases. `document.hasFocus()` returns false under jsdom, so the tab-away
 * escalation condition holds by default; the tab-VISIBLE case spies it to true.
 */
import { afterEach, describe, expect, it, vi } from "vitest";
import { act, renderHook } from "@testing-library/react";
import { useDesktopNotifications } from "../src/hooks/useDesktopNotifications";
import type { ManualInterventionReason, Notification } from "../src/data/contract";

interface Constructed {
  title: string;
  options: { body?: string; tag?: string };
}

/** Build a stub `window.Notification` class capturing each construction. */
function makeFakeNotification(
  permission: NotificationPermission,
  constructed: Constructed[],
  requestImpl?: () => Promise<NotificationPermission>,
): unknown {
  return class FakeNotification {
    static permission: NotificationPermission = permission;
    static requestPermission =
      requestImpl ?? vi.fn(() => Promise.resolve(permission));
    onclick: (() => void) | null = null;
    constructor(title: string, options: { body?: string; tag?: string }) {
      constructed.push({ title, options });
    }
    close(): void {}
  };
}

/** A manual-intervention notification carrying an alert-worthy reason. */
function manualNote(
  reason: ManualInterventionReason,
  detail?: string,
): Notification {
  return {
    notificationId: "ntf-mi",
    kind: "MANUAL_INTERVENTION_REQUIRED",
    atNanos: 1n,
    desk: "g10-rates",
    counterparty: "Meridian Capital",
    requestKind: "RFQ",
    headline: "Manual pricing needed",
    ...(detail !== undefined ? { detail } : {}),
    reason,
    alertWorthy: true,
  };
}

/** A plain alert-worthy RFQ notification. */
function rfqNote(): Notification {
  return {
    notificationId: "ntf-rfq",
    kind: "RFQ_RECEIVED",
    atNanos: 1n,
    desk: "g10-rates",
    counterparty: "ACME",
    requestKind: "RFQ",
    headline: "RFQ from ACME",
    detail: "5y OIS 75mm",
    alertWorthy: true,
  };
}

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
  try {
    window.localStorage.clear();
  } catch {
    /* ignore */
  }
});

describe("useDesktopNotifications — graceful degradation", () => {
  it("reports unsupported and no-ops when the browser lacks the API", () => {
    // jsdom has no window.Notification → this IS the unsupported case.
    const { result } = renderHook(() => useDesktopNotifications());
    expect(result.current.supported).toBe(false);
    expect(result.current.permission).toBe("unsupported");
    expect(result.current.enabled).toBe(false);
    // notify + toggle must be silent no-ops that never throw.
    expect(() => result.current.notify(rfqNote())).not.toThrow();
    expect(() => act(() => result.current.toggle())).not.toThrow();
  });

  it("does not construct an OS notification when permission is denied", () => {
    const constructed: Constructed[] = [];
    vi.stubGlobal("Notification", makeFakeNotification("denied", constructed));
    const { result } = renderHook(() => useDesktopNotifications());
    expect(result.current.enabled).toBe(false);
    expect(() => result.current.notify(rfqNote())).not.toThrow();
    expect(constructed).toHaveLength(0);
  });

  it("tolerates a throwing requestPermission (very old browsers)", () => {
    const constructed: Constructed[] = [];
    const throwing = vi.fn(() => {
      throw new Error("legacy requestPermission blew up");
    }) as unknown as () => Promise<NotificationPermission>;
    vi.stubGlobal(
      "Notification",
      makeFakeNotification("default", constructed, throwing),
    );
    const { result } = renderHook(() => useDesktopNotifications());
    // The first-mount auto-request already exercised requestPermission on mount;
    // an explicit toggle must also not throw.
    expect(() => act(() => result.current.toggle())).not.toThrow();
  });
});

describe("useDesktopNotifications — escalation + reason body", () => {
  it("does NOT double-notify when the tab is visible + focused", () => {
    const constructed: Constructed[] = [];
    vi.stubGlobal("Notification", makeFakeNotification("granted", constructed));
    vi.spyOn(document, "hasFocus").mockReturnValue(true); // tab on screen
    const { result } = renderHook(() => useDesktopNotifications());
    expect(result.current.enabled).toBe(true);
    result.current.notify(rfqNote());
    expect(constructed).toHaveLength(0);
  });

  it("constructs an OS notification when granted and the tab is away", () => {
    const constructed: Constructed[] = [];
    vi.stubGlobal("Notification", makeFakeNotification("granted", constructed));
    // hasFocus() is false under jsdom by default ⇒ tab away.
    const { result } = renderHook(() => useDesktopNotifications());
    result.current.notify(rfqNote());
    expect(constructed).toHaveLength(1);
    expect(constructed[0]?.title).toBe("RFQ from ACME");
    expect(constructed[0]?.options.body).toBe("5y OIS 75mm");
  });

  it("renders the manual-intervention reason in the OS body (detail + reason)", () => {
    const constructed: Constructed[] = [];
    vi.stubGlobal("Notification", makeFakeNotification("granted", constructed));
    const { result } = renderHook(() => useDesktopNotifications());
    result.current.notify(manualNote("UNCONFIGURED_TENOR", "USD-OIS 15Y"));
    expect(constructed).toHaveLength(1);
    expect(constructed[0]?.options.body).toBe(
      "USD-OIS 15Y — Manual pricing needed — unconfigured tenor",
    );
  });

  it("renders just the reason label when the manual-intervention detail is absent", () => {
    const constructed: Constructed[] = [];
    vi.stubGlobal("Notification", makeFakeNotification("granted", constructed));
    const { result } = renderHook(() => useDesktopNotifications());
    result.current.notify(manualNote("CREDIT_RISK_BREAK"));
    expect(constructed[0]?.options.body).toBe(
      "Manual pricing needed — credit risk break",
    );
  });
});
