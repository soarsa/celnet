import { describe, expect, it } from "vitest";

import {
  isNewerRelease,
  parseReleaseManifest,
  type ReleaseManifest,
} from "../src/data/versionManifest";

const running: ReleaseManifest = { hash: "89b0588", buildTime: "2026-06-27T13:00:00.000Z" };

describe("parseReleaseManifest", () => {
  it("accepts a well-formed manifest", () => {
    expect(parseReleaseManifest({ hash: "abc1234", buildTime: "2026-06-27T14:00:00.000Z" })).toEqual(
      { hash: "abc1234", buildTime: "2026-06-27T14:00:00.000Z" },
    );
  });

  it("ignores extra fields, keeping only hash + buildTime", () => {
    const parsed = parseReleaseManifest({
      hash: "abc1234",
      buildTime: "2026-06-27T14:00:00.000Z",
      release: "abc1234-20260627T140000Z",
    });
    expect(parsed).toEqual({ hash: "abc1234", buildTime: "2026-06-27T14:00:00.000Z" });
  });

  it("rejects missing or empty fields", () => {
    expect(parseReleaseManifest({ hash: "abc1234" })).toBeNull();
    expect(parseReleaseManifest({ buildTime: "2026-06-27T14:00:00.000Z" })).toBeNull();
    expect(parseReleaseManifest({ hash: "", buildTime: "2026-06-27T14:00:00.000Z" })).toBeNull();
    expect(parseReleaseManifest({ hash: "abc1234", buildTime: "" })).toBeNull();
  });

  it("rejects wrong types and non-objects", () => {
    expect(parseReleaseManifest({ hash: 42, buildTime: "2026-06-27T14:00:00.000Z" })).toBeNull();
    expect(parseReleaseManifest({ hash: "abc1234", buildTime: 42 })).toBeNull();
    expect(parseReleaseManifest(null)).toBeNull();
    expect(parseReleaseManifest("abc1234")).toBeNull();
    expect(parseReleaseManifest(undefined)).toBeNull();
    expect(parseReleaseManifest([])).toBeNull();
  });
});

describe("isNewerRelease", () => {
  it("is false for the identical running build", () => {
    expect(isNewerRelease(running, { ...running })).toBe(false);
  });

  it("is true when the git hash differs", () => {
    expect(isNewerRelease(running, { hash: "abc1234", buildTime: running.buildTime })).toBe(true);
  });

  it("is true for a rebuild of the same commit with a strictly later build time", () => {
    expect(
      isNewerRelease(running, { hash: running.hash, buildTime: "2026-06-27T13:30:00.000Z" }),
    ).toBe(true);
  });

  it("is false for the same hash with an equal or earlier build time", () => {
    expect(
      isNewerRelease(running, { hash: running.hash, buildTime: "2026-06-27T12:00:00.000Z" }),
    ).toBe(false);
  });

  it("orders ISO build times chronologically (lexicographic compare is valid)", () => {
    const older: ReleaseManifest = { hash: "x", buildTime: "2026-01-01T00:00:00.000Z" };
    const newer: ReleaseManifest = { hash: "x", buildTime: "2026-12-31T23:59:59.000Z" };
    expect(isNewerRelease(older, newer)).toBe(true);
    expect(isNewerRelease(newer, older)).toBe(false);
  });
});
