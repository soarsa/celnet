import { beforeEach, describe, expect, it } from "vitest";
import {
  DEFAULT_SETTINGS,
  DEFAULT_PER_EVENT,
  NOTIFICATION_EVENT_TYPES,
  SETTINGS_STORAGE_KEY,
  SETTINGS_STORAGE_KEY_V1,
  effectiveEventVolume,
  eventTypeForKind,
  loadSettings,
  saveSettings,
} from "../src/settings/settingsSchema";

describe("settingsSchema v2", () => {
  beforeEach(() => {
    window.localStorage.clear();
  });

  it("uses the v2 versioned storage key", () => {
    expect(SETTINGS_STORAGE_KEY).toBe("celnet.settings.v2");
    expect(SETTINGS_STORAGE_KEY_V1).toBe("celnet.settings.v1");
  });

  it("DEFAULT_SETTINGS carries the new masters + a full per-event map", () => {
    expect(DEFAULT_SETTINGS.masterMute).toBe(false);
    expect(DEFAULT_SETTINGS.masterVolume).toBe(70);
    expect(DEFAULT_SETTINGS.streakWindowMs).toBe(1500);
    expect(DEFAULT_SETTINGS.reducedMotion).toBe("auto");
    for (const et of NOTIFICATION_EVENT_TYPES) {
      expect(DEFAULT_SETTINGS.perEvent[et]).toBeDefined();
    }
  });

  it("returns defaults when nothing is stored", () => {
    expect(loadSettings()).toEqual(DEFAULT_SETTINGS);
  });

  it("returns defaults for garbage / non-object JSON", () => {
    window.localStorage.setItem(SETTINGS_STORAGE_KEY, "{not json");
    expect(loadSettings()).toEqual(DEFAULT_SETTINGS);
    window.localStorage.setItem(SETTINGS_STORAGE_KEY, "42");
    expect(loadSettings()).toEqual(DEFAULT_SETTINGS);
  });

  it("round-trips a full v2 object through save → load", () => {
    const s = saveable();
    saveSettings(s);
    expect(loadSettings()).toEqual(s);
  });

  describe("v1 → v2 migration", () => {
    it("carries v1 scalars forward, seeds masterVolume from v1 volume, defaults perEvent", () => {
      window.localStorage.setItem(
        SETTINGS_STORAGE_KEY_V1,
        JSON.stringify({
          alertsEnabled: true,
          soundsEnabled: false,
          volume: 25,
          minQty: 5_000_000,
          growlEnabled: false,
          autoClearCompleted: false,
          autoClearTtlSeconds: 30,
        }),
      );
      const loaded = loadSettings();
      // v1 scalars survive.
      expect(loaded.soundsEnabled).toBe(false);
      expect(loaded.minQty).toBe(5_000_000);
      expect(loaded.autoClearTtlSeconds).toBe(30);
      // masterVolume seeds from the v1 volume; masterMute defaults false.
      expect(loaded.masterVolume).toBe(25);
      expect(loaded.masterMute).toBe(false);
      // every per-event entry falls back to its default.
      for (const et of NOTIFICATION_EVENT_TYPES) {
        expect(loaded.perEvent[et]).toEqual(DEFAULT_PER_EVENT[et]);
      }
    });

    it("prefers a present v2 blob over a stale v1 blob", () => {
      window.localStorage.setItem(SETTINGS_STORAGE_KEY_V1, JSON.stringify({ volume: 10 }));
      window.localStorage.setItem(SETTINGS_STORAGE_KEY, JSON.stringify({ masterVolume: 90 }));
      expect(loadSettings().masterVolume).toBe(90);
    });
  });

  describe("per-event forward-safe merge", () => {
    it("fills a missing per-event entry with its default", () => {
      window.localStorage.setItem(
        SETTINGS_STORAGE_KEY,
        JSON.stringify({
          perEvent: {
            RfqReceived: { enabled: false, sound: "won", channels: { toast: false, desktop: true }, volume: 40 },
          },
        }),
      );
      const loaded = loadSettings();
      // The stored entry is honoured.
      expect(loaded.perEvent.RfqReceived).toEqual({
        enabled: false,
        sound: "won",
        channels: { toast: false, desktop: true },
        volume: 40,
      });
      // Entries absent from the blob default.
      expect(loaded.perEvent.QuoteAccepted).toEqual(DEFAULT_PER_EVENT.QuoteAccepted);
    });

    it("fills missing sub-fields of a partial per-event entry with defaults", () => {
      window.localStorage.setItem(
        SETTINGS_STORAGE_KEY,
        JSON.stringify({ perEvent: { QuoteAccepted: { volume: 20 } } }),
      );
      const p = loadSettings().perEvent.QuoteAccepted;
      expect(p.volume).toBe(20);
      expect(p.sound).toBe(DEFAULT_PER_EVENT.QuoteAccepted.sound);
      expect(p.channels).toEqual(DEFAULT_PER_EVENT.QuoteAccepted.channels);
      expect(p.enabled).toBe(true);
    });

    it("rejects an invalid sound id, falling back to the default cue", () => {
      window.localStorage.setItem(
        SETTINGS_STORAGE_KEY,
        JSON.stringify({ perEvent: { RfqReceived: { sound: "airhorn" } } }),
      );
      expect(loadSettings().perEvent.RfqReceived.sound).toBe(DEFAULT_PER_EVENT.RfqReceived.sound);
    });

    it("clamps a per-event volume into 0..100", () => {
      window.localStorage.setItem(
        SETTINGS_STORAGE_KEY,
        JSON.stringify({ perEvent: { Fill: { volume: 999 }, FillBlock: { volume: -5 } } }),
      );
      const loaded = loadSettings();
      expect(loaded.perEvent.Fill.volume).toBe(100);
      expect(loaded.perEvent.FillBlock.volume).toBe(0);
    });
  });

  it("clamps master/legacy volume and floors minQty", () => {
    window.localStorage.setItem(
      SETTINGS_STORAGE_KEY,
      JSON.stringify({ masterVolume: 999, volume: -20, minQty: -5 }),
    );
    const loaded = loadSettings();
    expect(loaded.masterVolume).toBe(100);
    expect(loaded.volume).toBe(0);
    expect(loaded.minQty).toBe(0);
  });

  it("ignores wrong-typed scalar fields", () => {
    window.localStorage.setItem(
      SETTINGS_STORAGE_KEY,
      JSON.stringify({ masterMute: "yes", masterVolume: "loud", reducedMotion: "wibble" }),
    );
    const loaded = loadSettings();
    expect(loaded.masterMute).toBe(DEFAULT_SETTINGS.masterMute);
    expect(loaded.masterVolume).toBe(DEFAULT_SETTINGS.masterVolume);
    expect(loaded.reducedMotion).toBe("auto");
  });
});

describe("eventTypeForKind", () => {
  it("maps every wire kind to a configurable event type", () => {
    expect(eventTypeForKind("RFQ_RECEIVED")).toBe("RfqReceived");
    expect(eventTypeForKind("IOI_RECEIVED")).toBe("IoiReceived");
    expect(eventTypeForKind("MANUAL_INTERVENTION_REQUIRED")).toBe("ManualIntervention");
    expect(eventTypeForKind("QUOTE_ACCEPTED")).toBe("QuoteAccepted");
    expect(eventTypeForKind("QUOTE_REJECTED")).toBe("QuoteRejected");
    expect(eventTypeForKind("REQUEST_WITHDRAWN")).toBe("RequestLapsed");
    expect(eventTypeForKind("REQUEST_EXPIRED")).toBe("RequestLapsed");
  });
});

describe("effectiveEventVolume", () => {
  it("scales the master volume by the per-event trim", () => {
    expect(effectiveEventVolume(80, 50)).toBe(40);
    expect(effectiveEventVolume(70, 100)).toBe(70);
  });

  it("clamps both inputs into 0..100", () => {
    expect(effectiveEventVolume(999, 100)).toBe(100);
    expect(effectiveEventVolume(80, -10)).toBe(0);
  });
});

/** A fully-populated v2 settings object for the round-trip test. */
function saveable() {
  return {
    ...DEFAULT_SETTINGS,
    masterMute: true,
    masterVolume: 33,
    minQty: 1_000_000,
    reducedMotion: "on" as const,
    perEvent: {
      ...DEFAULT_PER_EVENT,
      QuoteRejected: {
        enabled: false,
        sound: "celebrate" as const,
        channels: { toast: true, desktop: false },
        volume: 15,
      },
    },
  };
}
