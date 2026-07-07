import { afterEach, describe, expect, it, vi } from "vitest";
import {
  expireByTtl,
  isTerminalKind,
  planNotification,
  playCue,
  pruneOnTerminal,
  shouldSuppress,
} from "../src/hooks/useNotificationStore";
import { DEFAULT_SETTINGS, type AppSettings } from "../src/settings/settingsSchema";
import type { Notification, NotificationKind } from "../src/data/contract";

function note(
  kind: NotificationKind,
  opts: { headline?: string; detail?: string; requestId?: string; atNanos?: bigint; id?: string } = {},
): Notification {
  return {
    notificationId: opts.id ?? "n1",
    kind,
    atNanos: opts.atNanos ?? 0n,
    requestId: opts.requestId,
    desk: "rates",
    counterparty: "ACME",
    requestKind: "RFQ",
    headline: opts.headline ?? "RFQ from ACME",
    detail: opts.detail,
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
  it("below-min raises NOTHING — no item, toast, sound, or growl", () => {
    const n = note("RFQ_RECEIVED", { headline: "RFQ 1m" });
    const plan = planNotification(n, settings({ minQty: 5_000_000 }), false);
    expect(plan).toEqual({
      suppressed: true,
      addItem: false,
      toast: false,
      sound: false,
      growl: false,
      bumpUnread: false,
    });
  });

  it("a large event raises item + toast + sound + growl + unread", () => {
    const n = note("RFQ_RECEIVED", { headline: "RFQ 75mm" });
    const plan = planNotification(n, settings({ minQty: 5_000_000 }), false);
    expect(plan).toEqual({
      suppressed: false,
      addItem: true,
      toast: true,
      sound: true,
      growl: true,
      bumpUnread: true,
    });
  });

  it("honours the sound / growl preferences", () => {
    const n = note("RFQ_RECEIVED", { headline: "RFQ 75mm" });
    const plan = planNotification(n, settings({ soundsEnabled: false, growlEnabled: false }), false);
    expect(plan.sound).toBe(false);
    expect(plan.growl).toBe(false);
    expect(plan.addItem).toBe(true);
  });

  it("does not bump unread when the dropdown is open", () => {
    const n = note("RFQ_RECEIVED", { headline: "RFQ 75mm" });
    expect(planNotification(n, settings(), true).bumpUnread).toBe(false);
  });

  it("a terminal event does not raise a toast", () => {
    const n = note("QUOTE_ACCEPTED", { headline: "accepted 75mm" });
    expect(planNotification(n, settings(), false).toast).toBe(false);
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
