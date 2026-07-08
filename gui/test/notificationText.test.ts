import { describe, expect, it } from "vitest";
import {
  compactNotionals,
  manualInterventionText,
  notionalMagnitude,
} from "../src/lib/notificationText";
import type { ManualInterventionReason, Notification } from "../src/data/contract";

/** A minimal Notification carrying just the text fields the heuristic scans. */
function note(headline: string, detail?: string): Notification {
  return {
    notificationId: "n1",
    kind: "RFQ_RECEIVED",
    atNanos: 0n,
    desk: "rates",
    counterparty: "ACME",
    requestKind: "RFQ",
    headline,
    detail,
    alertWorthy: true,
  };
}

describe("notionalMagnitude", () => {
  it("reads a mm-suffixed size (mock form)", () => {
    expect(notionalMagnitude(note("RFQ from ACME: 5y OIS 75mm"))).toBe(75_000_000);
  });

  it("reads a raw '<n> notional' size (live-server form)", () => {
    expect(notionalMagnitude(note("10000000 notional"))).toBe(10_000_000);
  });

  it("reads m/k/b/t suffixes and strips commas", () => {
    expect(notionalMagnitude(note("1.5m"))).toBe(1_500_000);
    expect(notionalMagnitude(note("1,500,000"))).toBe(1_500_000);
    expect(notionalMagnitude(note("250k"))).toBe(250_000);
    expect(notionalMagnitude(note("2bn"))).toBe(2_000_000_000);
    expect(notionalMagnitude(note("3b"))).toBe(3_000_000_000);
    expect(notionalMagnitude(note("1t"))).toBe(1_000_000_000_000);
  });

  it("returns undefined for a percentage/rate", () => {
    expect(notionalMagnitude(note("3.250%"))).toBeUndefined();
  });

  it("returns undefined for a tenor", () => {
    expect(notionalMagnitude(note("5y OIS"))).toBeUndefined();
  });

  it("ignores small bare ids but reads the real size in the same string", () => {
    expect(notionalMagnitude(note("RFQ from ACME: 5y OIS 75mm", "deal deal-1 · 75mm"))).toBe(
      75_000_000,
    );
  });

  it("scans headline then detail, returning the FIRST notional", () => {
    expect(notionalMagnitude(note("no size here", "then 30mm later"))).toBe(30_000_000);
  });

  it("is fail-open (undefined) when nothing parses", () => {
    expect(notionalMagnitude(note("quote accepted"))).toBeUndefined();
  });
});

describe("compactNotionals", () => {
  it("compacts a raw notional and preserves the ' notional' word", () => {
    expect(compactNotionals("10000000 notional")).toBe("10m notional");
  });

  it("compacts an mm form", () => {
    expect(compactNotionals("75mm")).toBe("75m");
  });

  it("compacts a comma-grouped raw number", () => {
    expect(compactNotionals("1,500,000")).toBe("1.5m");
  });

  it("leaves a percentage untouched", () => {
    expect(compactNotionals("3.250%")).toBe("3.250%");
  });

  it("leaves a tenor untouched", () => {
    expect(compactNotionals("5y OIS")).toBe("5y OIS");
  });

  it("compacts only the notional inside a mixed headline", () => {
    expect(compactNotionals("RFQ from ACME: 5y OIS 75mm")).toBe(
      "RFQ from ACME: 5y OIS 75m",
    );
  });

  it("does not touch a large number immediately followed by %", () => {
    expect(compactNotionals("1500%")).toBe("1500%");
  });
});

describe("manualInterventionText", () => {
  it("maps each ManualInterventionReason ordinal (1–4) to its label", () => {
    const cases: Array<[ManualInterventionReason, string]> = [
      ["UNCONFIGURED_TENOR", "Manual pricing needed — unconfigured tenor"],
      ["CREDIT_RISK_BREAK", "Manual pricing needed — credit risk break"],
      ["UNKNOWN_SECURITY", "Manual pricing needed — unknown security"],
      ["PRICING_FAILURE", "Manual pricing needed — pricing failure"],
    ];
    for (const [reason, label] of cases) {
      expect(manualInterventionText(reason)).toBe(label);
    }
  });
});
