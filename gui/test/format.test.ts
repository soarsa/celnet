/**
 * fmtCompact — the compact-magnitude formatter used across the blotters, tickets
 * and risk views so a large counting quantity (notional, size, whole-currency
 * amount) reads as "10m" rather than "10,000,000". These pin the exact suffixing,
 * the k/b boundaries, sign handling, and the non-finite em-dash contract.
 */

import { describe, expect, it } from "vitest";
import { fmtCompact } from "../src/lib/format";

describe("fmtCompact", () => {
  it("suffixes millions", () => {
    expect(fmtCompact(1_000_000)).toBe("1m");
    expect(fmtCompact(50_000_000)).toBe("50m");
    expect(fmtCompact(10_000_000)).toBe("10m");
  });

  it("suffixes thousands", () => {
    expect(fmtCompact(100_000)).toBe("100k");
    expect(fmtCompact(1_234)).toBe("1.23k");
  });

  it("suffixes billions with fractional digits", () => {
    expect(fmtCompact(1_250_000_000)).toBe("1.25b");
  });

  it("leaves sub-thousand magnitudes bare", () => {
    expect(fmtCompact(750)).toBe("750");
    expect(fmtCompact(0)).toBe("0");
  });

  it("handles the k boundary", () => {
    expect(fmtCompact(999)).toBe("999");
    expect(fmtCompact(1_000)).toBe("1k");
  });

  it("keeps the sign on negatives", () => {
    expect(fmtCompact(-1_000_000)).toBe("-1m");
    expect(fmtCompact(-1_234)).toBe("-1.23k");
  });

  it("renders non-finite values as an em dash", () => {
    expect(fmtCompact(Number.NaN)).toBe("—");
    expect(fmtCompact(Number.POSITIVE_INFINITY)).toBe("—");
    expect(fmtCompact(Number.NEGATIVE_INFINITY)).toBe("—");
  });
});
