import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import {
  alreadyReloadedFor,
  clearClientCaches,
  isNewerRelease,
  markReloadedFor,
  parseReleaseManifest,
  RELOAD_GUARD_KEY,
  resetAndReloadTo,
  type ReleaseManifest,
} from "../src/data/versionManifest";

const running: ReleaseManifest = { hash: "v0.0.0", buildTime: "2026-06-27T13:00:00.000Z" };

/** A standalone, Map-backed `Storage` so guard tests never touch real sessionStorage. */
function fakeStore(): Storage {
  const m = new Map<string, string>();
  return {
    get length() {
      return m.size;
    },
    clear: () => m.clear(),
    getItem: (k: string) => (m.has(k) ? (m.get(k) as string) : null),
    key: (i: number) => Array.from(m.keys())[i] ?? null,
    removeItem: (k: string) => void m.delete(k),
    setItem: (k: string, v: string) => void m.set(k, v),
  } as Storage;
}

describe("parseReleaseManifest", () => {
  it("accepts a well-formed manifest", () => {
    expect(parseReleaseManifest({ hash: "v0.0.0", buildTime: "2026-06-27T14:00:00.000Z" })).toEqual({
      hash: "v0.0.0",
      buildTime: "2026-06-27T14:00:00.000Z",
    });
  });

  it("ignores extra fields, keeping only hash + buildTime", () => {
    const parsed = parseReleaseManifest({
      hash: "v0.0.0",
      buildTime: "2026-06-27T14:00:00.000Z",
      release: "v0.0.0-20260627T140000Z",
    });
    expect(parsed).toEqual({ hash: "v0.0.0", buildTime: "2026-06-27T14:00:00.000Z" });
  });

  it("rejects missing or empty fields", () => {
    expect(parseReleaseManifest({ hash: "v0.0.0" })).toBeNull();
    expect(parseReleaseManifest({ buildTime: "2026-06-27T14:00:00.000Z" })).toBeNull();
    expect(parseReleaseManifest({ hash: "", buildTime: "2026-06-27T14:00:00.000Z" })).toBeNull();
    expect(parseReleaseManifest({ hash: "v0.0.0", buildTime: "" })).toBeNull();
  });

  it("rejects wrong types and non-objects", () => {
    expect(parseReleaseManifest({ hash: 42, buildTime: "2026-06-27T14:00:00.000Z" })).toBeNull();
    expect(parseReleaseManifest({ hash: "v0.0.0", buildTime: 42 })).toBeNull();
    expect(parseReleaseManifest(null)).toBeNull();
    expect(parseReleaseManifest("v0.0.0")).toBeNull();
    expect(parseReleaseManifest(undefined)).toBeNull();
    expect(parseReleaseManifest([])).toBeNull();
  });
});

describe("isNewerRelease — keyed on buildTime only (hash is a deploy constant)", () => {
  it("is false for the identical running build", () => {
    expect(isNewerRelease(running, { ...running })).toBe(false);
  });

  it("ignores a differing hash when the buildTime is equal (deploy hash is always v0.0.0)", () => {
    // A different hash with the SAME buildTime must NOT count as newer — else the
    // constant-`v0.0.0` deploy hash would false-positive a reload every poll.
    expect(isNewerRelease(running, { hash: "abc1234", buildTime: running.buildTime })).toBe(false);
  });

  it("is true for a rebuild with a strictly later build time", () => {
    expect(
      isNewerRelease(running, { hash: running.hash, buildTime: "2026-06-27T13:30:00.000Z" }),
    ).toBe(true);
  });

  it("is false for an equal or earlier build time", () => {
    expect(
      isNewerRelease(running, { hash: running.hash, buildTime: "2026-06-27T12:00:00.000Z" }),
    ).toBe(false);
  });

  it("orders ISO build times chronologically (lexicographic compare is valid)", () => {
    const older: ReleaseManifest = { hash: "v0.0.0", buildTime: "2026-01-01T00:00:00.000Z" };
    const newer: ReleaseManifest = { hash: "v0.0.0", buildTime: "2026-12-31T23:59:59.000Z" };
    expect(isNewerRelease(older, newer)).toBe(true);
    expect(isNewerRelease(newer, older)).toBe(false);
  });
});

describe("reload loop-guard (sessionStorage-keyed, once per detected build)", () => {
  it("records and reports a reload per exact buildTime", () => {
    const store = fakeStore();
    expect(alreadyReloadedFor("2026-06-27T14:00:00.000Z", store)).toBe(false);
    markReloadedFor("2026-06-27T14:00:00.000Z", store);
    expect(store.getItem(RELOAD_GUARD_KEY)).toBe("2026-06-27T14:00:00.000Z");
    expect(alreadyReloadedFor("2026-06-27T14:00:00.000Z", store)).toBe(true);
    // A DIFFERENT (even newer) build is not yet reloaded-for.
    expect(alreadyReloadedFor("2026-06-27T15:00:00.000Z", store)).toBe(false);
  });
});

describe("resetAndReloadTo — cache-busting reload, idempotent per build", () => {
  const target: ReleaseManifest = { hash: "v0.0.0", buildTime: "2026-06-27T14:00:00.000Z" };

  it("clears caches then reloads exactly once for a new build, and sets the guard", async () => {
    const store = fakeStore();
    const reload = vi.fn();
    const clearCaches = vi.fn(async () => {});

    const order: string[] = [];
    clearCaches.mockImplementation(async () => void order.push("clear"));
    reload.mockImplementation(() => void order.push("reload"));

    const did = await resetAndReloadTo(target, { store, reload, clearCaches });

    expect(did).toBe(true);
    expect(clearCaches).toHaveBeenCalledTimes(1);
    expect(reload).toHaveBeenCalledTimes(1);
    // Caches are cleared BEFORE the reload fires.
    expect(order).toEqual(["clear", "reload"]);
    expect(alreadyReloadedFor(target.buildTime, store)).toBe(true);
  });

  it("does NOT reload again on a second call for the SAME build (no loop)", async () => {
    const store = fakeStore();
    const reload = vi.fn();
    const clearCaches = vi.fn(async () => {});

    await resetAndReloadTo(target, { store, reload, clearCaches });
    const second = await resetAndReloadTo(target, { store, reload, clearCaches });

    expect(second).toBe(false);
    expect(reload).toHaveBeenCalledTimes(1); // still ONE
    expect(clearCaches).toHaveBeenCalledTimes(1);
  });

  it("reloads again for a DISTINCT newer build (guard is per-buildTime)", async () => {
    const store = fakeStore();
    const reload = vi.fn();
    const clearCaches = vi.fn(async () => {});
    const newer: ReleaseManifest = { hash: "v0.0.0", buildTime: "2026-06-27T15:00:00.000Z" };

    await resetAndReloadTo(target, { store, reload, clearCaches });
    const did = await resetAndReloadTo(newer, { store, reload, clearCaches });

    expect(did).toBe(true);
    expect(reload).toHaveBeenCalledTimes(2);
  });
});

describe("clearClientCaches — best-effort, never throws", () => {
  const origCaches = (globalThis as { caches?: unknown }).caches;

  afterEach(() => {
    if (origCaches === undefined) delete (globalThis as { caches?: unknown }).caches;
    else (globalThis as { caches?: unknown }).caches = origCaches;
    vi.restoreAllMocks();
  });

  it("deletes every Cache Storage entry", async () => {
    const del = vi.fn(async () => true);
    (globalThis as { caches?: unknown }).caches = {
      keys: vi.fn(async () => ["celnet-v1", "assets"]),
      delete: del,
    };

    await clearClientCaches();

    expect(del).toHaveBeenCalledWith("celnet-v1");
    expect(del).toHaveBeenCalledWith("assets");
    expect(del).toHaveBeenCalledTimes(2);
  });

  it("swallows a throwing CacheStorage without rejecting", async () => {
    (globalThis as { caches?: unknown }).caches = {
      keys: vi.fn(async () => {
        throw new Error("blocked");
      }),
      delete: vi.fn(),
    };
    await expect(clearClientCaches()).resolves.toBeUndefined();
  });
});
