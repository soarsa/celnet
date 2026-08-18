import { afterEach, describe, expect, it, vi } from "vitest";
import {
  advanceStreak,
  EMPTY_STREAK,
  expireByTtl,
  isFillKind,
  isTerminalKind,
  planNotification,
  playCue,
  pruneOnTerminal,
  shouldSuppress,
} from "../src/hooks/useNotificationStore";
import { tabIsAway } from "../src/hooks/useDesktopNotifications";
import {
  DEFAULT_SETTINGS,
  DEFAULT_PER_EVENT,
  type AppSettings,
} from "../src/settings/settingsSchema";
import type { Notification, NotificationKind } from "../src/data/contract";

function note(
  kind: NotificationKind,
  opts: {
    headline?: string;
    detail?: string;
    requestId?: string;
    atNanos?: bigint;
    id?: string;
    alertWorthy?: boolean;
  } = {},
): Notification {
  return {
    notificationId: opts.id ?? "n1",
    kind,
    atNanos: opts.atNanos ?? 0n,
    ...(opts.requestId !== undefined ? { requestId: opts.requestId } : {}),
    desk: "rates",
    counterparty: "ACME",
    requestKind: "RFQ",
    headline: opts.headline ?? "RFQ from ACME",
    ...(opts.detail !== undefined ? { detail: opts.detail } : {}),
    // Default alert-worthy so the pre-existing gating tests keep exercising the
    // popup path; the quiet-path tests below pass `alertWorthy: false` explicitly.
    alertWorthy: opts.alertWorthy ?? true,
  };
}

const settings = (patch: Partial<AppSettings> = {}): AppSettings => ({
  ...DEFAULT_SETTINGS,
  ...patch,
});

describe("shouldSuppress", () => {
  it("suppresses everything when master alerts are off", () => {
    expect(shouldSuppress(note("RFQ_RECEIVED", { headline: "75mm" }), settings({ alertsEnabled: false }))).toBe(
      true,
    );
  });

  it("suppresses an event whose notional is below minQty", () => {
    const n = note("RFQ_RECEIVED", { headline: "RFQ 1m" }); // 1,000,000
    expect(shouldSuppress(n, settings({ minQty: 5_000_000 }))).toBe(true);
  });

  it("does NOT suppress an event at/above minQty", () => {
    const n = note("RFQ_RECEIVED", { headline: "RFQ 10m" });
    expect(shouldSuppress(n, settings({ minQty: 5_000_000 }))).toBe(false);
  });

  it("is fail-open when the size cannot be parsed (never suppress by threshold)", () => {
    const n = note("QUOTE_ACCEPTED", { headline: "quote accepted" });
    expect(shouldSuppress(n, settings({ minQty: 5_000_000 }))).toBe(false);
  });
});

describe("planNotification", () => {
  // isOpen=false, isAway=false unless a test overrides — the on-screen default.
  it("below-min raises NOTHING — no item, toast, desktop, or sound", () => {
    const n = note("RFQ_RECEIVED", { headline: "RFQ 1m" });
    const plan = planNotification(n, settings({ minQty: 5_000_000 }), false, false);
    expect(plan).toEqual({
      suppressed: true,
      addItem: false,
      toast: false,
      desktop: false,
      sound: "none",
      volume: 0,
      bumpUnread: false,
    });
  });

  it("an on-screen RFQ raises item + toast + its configured sound, no desktop", () => {
    const n = note("RFQ_RECEIVED", { headline: "RFQ 75mm" });
    const plan = planNotification(n, settings({ minQty: 5_000_000 }), false, false);
    expect(plan).toEqual({
      suppressed: false,
      addItem: true,
      toast: true, // tab on screen ⇒ in-app growl toast
      desktop: false, // RfqReceived default channel is toast-only
      sound: "rfq-work", // the configured cue for this event
      volume: DEFAULT_SETTINGS.masterVolume, // master × 100% trim
      bumpUnread: true,
    });
  });

  it("picks each event's configured sound", () => {
    const s = settings();
    expect(planNotification(note("QUOTE_ACCEPTED", { headline: "won 75mm" }), s, false, false).sound).toBe("won");
    expect(planNotification(note("QUOTE_REJECTED", { headline: "lost 75mm" }), s, false, false).sound).toBe("lost");
    expect(planNotification(note("MANUAL_INTERVENTION_REQUIRED", { headline: "help" }), s, false, false).sound).toBe(
      "needs-you",
    );
    expect(planNotification(note("REQUEST_EXPIRED", { headline: "lapsed 75mm" }), s, false, false).sound).toBe("lapsed");
  });

  it("focus-aware routing: toast when on screen, desktop when away — never both", () => {
    // ManualIntervention defaults to BOTH channels.
    const n = note("MANUAL_INTERVENTION_REQUIRED", { headline: "manual", alertWorthy: true });
    const onScreen = planNotification(n, settings(), false, false);
    expect(onScreen.toast).toBe(true);
    expect(onScreen.desktop).toBe(false);
    const away = planNotification(n, settings(), false, true);
    expect(away.toast).toBe(false);
    expect(away.desktop).toBe(true);
  });

  it("a toast-only event stays silent on the desktop channel even when away", () => {
    // RfqReceived default is toast-only.
    const n = note("RFQ_RECEIVED", { headline: "RFQ 75mm" });
    const away = planNotification(n, settings(), false, true);
    expect(away.toast).toBe(false); // tab away ⇒ no toast
    expect(away.desktop).toBe(false); // desktop channel off for this event
  });

  it("masterMute silences the cue but keeps the toast/item", () => {
    const n = note("RFQ_RECEIVED", { headline: "RFQ 75mm" });
    const plan = planNotification(n, settings({ masterMute: true }), false, false);
    expect(plan.sound).toBe("none");
    expect(plan.volume).toBe(0);
    expect(plan.toast).toBe(true);
    expect(plan.addItem).toBe(true);
  });

  it("soundsEnabled=false silences the cue", () => {
    const n = note("RFQ_RECEIVED", { headline: "RFQ 75mm" });
    expect(planNotification(n, settings({ soundsEnabled: false }), false, false).sound).toBe("none");
  });

  it("the growl master gates the desktop channel", () => {
    const n = note("MANUAL_INTERVENTION_REQUIRED", { headline: "manual" });
    const away = planNotification(n, settings({ growlEnabled: false }), false, true);
    expect(away.desktop).toBe(false);
  });

  it("a per-event volume trim scales the master volume", () => {
    const perEvent = {
      ...DEFAULT_PER_EVENT,
      RfqReceived: { ...DEFAULT_PER_EVENT.RfqReceived, volume: 50 },
    };
    const n = note("RFQ_RECEIVED", { headline: "RFQ 75mm" });
    const plan = planNotification(n, settings({ perEvent, masterVolume: 80 }), false, false);
    expect(plan.volume).toBe(40); // 80 × 50%
  });

  it("a per-event disabled toggle keeps the item but raises no popup/sound", () => {
    const perEvent = {
      ...DEFAULT_PER_EVENT,
      RfqReceived: { ...DEFAULT_PER_EVENT.RfqReceived, enabled: false },
    };
    const n = note("RFQ_RECEIVED", { headline: "RFQ 75mm" });
    const plan = planNotification(n, settings({ perEvent }), false, false);
    expect(plan.addItem).toBe(true);
    expect(plan.toast).toBe(false);
    expect(plan.desktop).toBe(false);
    expect(plan.sound).toBe("none");
  });

  it("does not bump unread when the dropdown is open", () => {
    const n = note("RFQ_RECEIVED", { headline: "RFQ 75mm" });
    expect(planNotification(n, settings(), true, false).bumpUnread).toBe(false);
  });

  it("a quiet (alertWorthy:false) event lands in the centre but raises NO popup", () => {
    const n = note("QUOTE_ACCEPTED", { headline: "auto-priced 75mm", alertWorthy: false });
    const plan = planNotification(n, settings(), false, false);
    expect(plan.suppressed).toBe(false);
    expect(plan.addItem).toBe(true);
    expect(plan.bumpUnread).toBe(true);
    expect(plan.toast).toBe(false);
    expect(plan.desktop).toBe(false);
    expect(plan.sound).toBe("none");
  });
});

describe("focus-aware tabIsAway", () => {
  const origHasFocus = document.hasFocus.bind(document);
  afterEach(() => {
    document.hasFocus = origHasFocus;
    Object.defineProperty(document, "visibilityState", { value: "visible", configurable: true });
  });

  it("is away when the document is hidden", () => {
    document.hasFocus = () => true;
    Object.defineProperty(document, "visibilityState", { value: "hidden", configurable: true });
    expect(tabIsAway()).toBe(true);
  });

  it("is away when the document is unfocused", () => {
    document.hasFocus = () => false;
    Object.defineProperty(document, "visibilityState", { value: "visible", configurable: true });
    expect(tabIsAway()).toBe(true);
  });

  it("is present when visible AND focused", () => {
    document.hasFocus = () => true;
    Object.defineProperty(document, "visibilityState", { value: "visible", configurable: true });
    expect(tabIsAway()).toBe(false);
  });
});

describe("fill-streak coalescing", () => {
  it("classifies the booked-deal QUOTE_ACCEPTED as a fill", () => {
    expect(isFillKind("QUOTE_ACCEPTED")).toBe(true);
    expect(isFillKind("RFQ_RECEIVED")).toBe(false);
  });

  it("increments within the window and restarts outside it", () => {
    const s1 = advanceStreak(EMPTY_STREAK, 1_000, 1_500); // fresh
    expect(s1.count).toBe(1);
    const s2 = advanceStreak(s1, 2_000, 1_500); // +1s, within
    expect(s2.count).toBe(2);
    const s3 = advanceStreak(s2, 3_000, 1_500); // +1s, within
    expect(s3.count).toBe(3);
    const s4 = advanceStreak(s3, 10_000, 1_500); // +7s, outside ⇒ reset
    expect(s4.count).toBe(1);
  });
});

describe("pruneOnTerminal (auto-clear: terminal-linkage)", () => {
  it("removes a pending notification when its terminal resolution arrives", () => {
    const items = [
      note("RFQ_RECEIVED", { id: "a", requestId: "req-1" }),
      note("RFQ_RECEIVED", { id: "b", requestId: "req-2" }),
    ];
    const incoming = note("QUOTE_ACCEPTED", { id: "c", requestId: "req-1" });
    const next = pruneOnTerminal(items, incoming);
    expect(next.map((n) => n.notificationId)).toEqual(["b"]);
  });

  it("is a no-op for a non-terminal inbound", () => {
    const items = [note("RFQ_RECEIVED", { id: "a", requestId: "req-1" })];
    const next = pruneOnTerminal(items, note("IOI_RECEIVED", { id: "z", requestId: "req-1" }));
    expect(next.map((n) => n.notificationId)).toEqual(["a"]);
  });

  it("is a no-op when the terminal has no requestId", () => {
    const items = [note("RFQ_RECEIVED", { id: "a", requestId: "req-1" })];
    const next = pruneOnTerminal(items, note("REQUEST_EXPIRED", { id: "z" }));
    expect(next).toHaveLength(1);
  });
});

describe("expireByTtl (auto-clear: TTL sweep)", () => {
  const now = 100_000n * 1_000_000_000n; // 100000 seconds, in ns

  it("drops items older than the TTL and keeps fresh ones", () => {
    const fresh = note("RFQ_RECEIVED", { id: "fresh", atNanos: now - 10n * 1_000_000_000n }); // 10s old
    const stale = note("RFQ_RECEIVED", { id: "stale", atNanos: now - 90n * 1_000_000_000n }); // 90s old
    const next = expireByTtl([fresh, stale], now, 60);
    expect(next.map((n) => n.notificationId)).toEqual(["fresh"]);
  });

  it("disables the sweep for a non-positive TTL", () => {
    const stale = note("RFQ_RECEIVED", { id: "stale", atNanos: 0n });
    expect(expireByTtl([stale], now, 0)).toHaveLength(1);
  });
});

describe("isTerminalKind", () => {
  it("classifies the four terminal kinds", () => {
    expect(isTerminalKind("QUOTE_ACCEPTED")).toBe(true);
    expect(isTerminalKind("QUOTE_REJECTED")).toBe(true);
    expect(isTerminalKind("REQUEST_WITHDRAWN")).toBe(true);
    expect(isTerminalKind("REQUEST_EXPIRED")).toBe(true);
    expect(isTerminalKind("RFQ_RECEIVED")).toBe(false);
  });
});

describe("playCue (WebAudio, guarded)", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("never throws when AudioContext is unavailable", () => {
    vi.stubGlobal("AudioContext", undefined);
    expect(() => playCue(60, false)).not.toThrow();
  });

  it("builds an oscillator cue when AudioContext is available", () => {
    const start = vi.fn();
    const stop = vi.fn();
    const connect = vi.fn(() => ({ connect: vi.fn() }));
    const gainConnect = vi.fn();
    const osc = { type: "", frequency: { value: 0 }, connect, start, stop };
    const gain = {
      gain: {
        setValueAtTime: vi.fn(),
        exponentialRampToValueAtTime: vi.fn(),
      },
      connect: gainConnect,
    };
    osc.connect = vi.fn(() => gain);
    class FakeAudioContext {
      state = "running";
      currentTime = 0;
      createOscillator() {
        return osc;
      }
      createGain() {
        return gain;
      }
      resume() {
        return Promise.resolve();
      }
    }
    vi.stubGlobal("AudioContext", FakeAudioContext);
    expect(() => playCue(80, false)).not.toThrow();
  });

  it("is a silent no-op at zero volume", () => {
    const create = vi.fn();
    class FakeAudioContext {
      state = "running";
      currentTime = 0;
      createOscillator() {
        create();
        return {};
      }
      createGain() {
        return {};
      }
    }
    vi.stubGlobal("AudioContext", FakeAudioContext);
    playCue(0, false);
    expect(create).not.toHaveBeenCalled();
  });
});
