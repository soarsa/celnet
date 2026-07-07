import { beforeEach, describe, expect, it } from "vitest";
import {
  DEFAULT_SETTINGS,
  SETTINGS_STORAGE_KEY,
  loadSettings,
  saveSettings,
  type AppSettings,
} from "../src/settings/settingsSchema";

describe("settingsSchema", () => {
  beforeEach(() => {
    window.localStorage.clear();
  });

  it("DEFAULT_SETTINGS has the documented shape and values", () => {
    expect(DEFAULT_SETTINGS).toEqual({
      alertsEnabled: true,
      soundsEnabled: true,
      volume: 60,
      minQty: 0,
      growlEnabled: true,
      autoClearCompleted: true,
      autoClearTtlSeconds: 60,
    });
  });

  it("uses the versioned storage key", () => {
    expect(SETTINGS_STORAGE_KEY).toBe("celnet.settings.v1");
  });

  it("returns defaults when the key is absent", () => {
    expect(loadSettings()).toEqual(DEFAULT_SETTINGS);
  });

  it("returns defaults when the stored value is garbage", () => {
    window.localStorage.setItem(SETTINGS_STORAGE_KEY, "{not json");
    expect(loadSettings()).toEqual(DEFAULT_SETTINGS);
  });

  it("returns defaults when the stored value is a non-object JSON", () => {
    window.localStorage.setItem(SETTINGS_STORAGE_KEY, "42");
    expect(loadSettings()).toEqual(DEFAULT_SETTINGS);
  });

  it("round-trips a full settings object through save → load", () => {
    const s: AppSettings = {
      alertsEnabled: false,
      soundsEnabled: false,
      volume: 25,
      minQty: 5_000_000,
      growlEnabled: false,
      autoClearCompleted: false,
      autoClearTtlSeconds: 30,
    };
    saveSettings(s);
    expect(loadSettings()).toEqual(s);
  });

  it("merges a partial stored blob over the defaults", () => {
    window.localStorage.setItem(
      SETTINGS_STORAGE_KEY,
      JSON.stringify({ volume: 10, minQty: 1_000_000 }),
    );
    const loaded = loadSettings();
    expect(loaded.volume).toBe(10);
    expect(loaded.minQty).toBe(1_000_000);
    // Fields absent from the blob fall back to the defaults.
    expect(loaded.alertsEnabled).toBe(DEFAULT_SETTINGS.alertsEnabled);
    expect(loaded.autoClearTtlSeconds).toBe(DEFAULT_SETTINGS.autoClearTtlSeconds);
  });

  it("clamps volume into 0..100 and floors minQty at 0", () => {
    window.localStorage.setItem(
      SETTINGS_STORAGE_KEY,
      JSON.stringify({ volume: 999, minQty: -5 }),
    );
    const hi = loadSettings();
    expect(hi.volume).toBe(100);
    expect(hi.minQty).toBe(0);

    window.localStorage.setItem(SETTINGS_STORAGE_KEY, JSON.stringify({ volume: -20 }));
    expect(loadSettings().volume).toBe(0);
  });

  it("ignores wrong-typed fields, keeping defaults", () => {
    window.localStorage.setItem(
      SETTINGS_STORAGE_KEY,
      JSON.stringify({ alertsEnabled: "yes", volume: "loud" }),
    );
    const loaded = loadSettings();
    expect(loaded.alertsEnabled).toBe(DEFAULT_SETTINGS.alertsEnabled);
    expect(loaded.volume).toBe(DEFAULT_SETTINGS.volume);
  });
});
