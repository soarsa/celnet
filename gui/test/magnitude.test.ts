import { describe, expect, it } from "vitest";

import {
  formatMagnitudeEcho,
  formatMagnitudeInput,
  parseMagnitude,
} from "../src/lib/magnitude";

/** Assert a successful parse and return the value, so the union stays honest. */
function value(raw: string): number {
  const result = parseMagnitude(raw);
  if (result.kind !== "ok") {
    throw new Error(`expected "${raw}" to parse, got ${result.kind}`);
  }
  return result.value;
}

/** Assert a rejection and return the message shown to the trader. */
function rejection(raw: string): string {
  const result = parseMagnitude(raw);
  if (result.kind !== "error") {
    throw new Error(`expected "${raw}" to be rejected, got ${result.kind}`);
  }
  return result.message;
}

describe("parseMagnitude — suffixes", () => {
  it("scales k, m and b", () => {
    expect(value("10k")).toBe(10_000);
    expect(value("5m")).toBe(5_000_000);
    expect(value("1b")).toBe(1_000_000_000);
  });

  it("accepts bn as an alias for b", () => {
    expect(value("1bn")).toBe(1_000_000_000);
    expect(value("2.5bn")).toBe(2_500_000_000);
  });

  it("is case-insensitive", () => {
    expect(value("1K")).toBe(1_000);
    expect(value("1M")).toBe(1_000_000);
    expect(value("1B")).toBe(1_000_000_000);
    expect(value("1BN")).toBe(1_000_000_000);
    expect(value("1Bn")).toBe(1_000_000_000);
    expect(value("1bN")).toBe(1_000_000_000);
  });

  it("reaches the hundred-billion limit that motivated the feature", () => {
    expect(value("100b")).toBe(100_000_000_000);
    expect(value("100000000000")).toBe(100_000_000_000);
    expect(value("100b")).toBe(value("100000000000"));
  });
});

describe("parseMagnitude — decimals are exact", () => {
  it("scales fractional mantissas without float drift", () => {
    expect(value("5.5m")).toBe(5_500_000);
    expect(value("1.25b")).toBe(1_250_000_000);
    expect(value("0.5k")).toBe(500);
  });

  it("produces exact integers where a float multiply would not", () => {
    // These are the cases that make the decimal-point shift earn its keep: on
    // IEEE-754 the naive `Number(mantissa) * factor` lands just under the
    // integer, which is precisely the `5499999.9999` class of error a risk
    // limit must never exhibit. Each case asserts BOTH that the naive route is
    // genuinely wrong and that the parser is right, so the test cannot quietly
    // decay into a tautology if the arithmetic ever changes.
    const drifting: ReadonlyArray<readonly [string, number, number]> = [
      ["1.001k", 1e3, 1_001],
      ["1.003k", 1e3, 1_003],
      ["1.005k", 1e3, 1_005],
      ["1.007k", 1e3, 1_007],
    ];

    for (const [raw, factor, exact] of drifting) {
      const mantissa = Number(raw.slice(0, -1));
      expect(mantissa * factor).not.toBe(exact);
      expect(value(raw)).toBe(exact);
      expect(Number.isInteger(value(raw))).toBe(true);
    }
  });

  it("keeps ordinary fractional shorthand exact too", () => {
    expect(value("5.5m")).toBe(5_500_000);
    expect(value("1.1b")).toBe(1_100_000_000);
    expect(value("2.675m")).toBe(2_675_000);
    expect(value("0.07b")).toBe(70_000_000);
    expect(value("8.2k")).toBe(8_200);
  });

  it("keeps sub-unit results exact", () => {
    expect(value("0.0005k")).toBe(0.5);
    expect(value(".5m")).toBe(500_000);
  });

  it("allows a trailing decimal point, as the native input does", () => {
    expect(value("5.")).toBe(5);
    expect(value("5.m")).toBe(5_000_000);
  });
});

describe("parseMagnitude — plain numbers pass through unchanged", () => {
  it("preserves the value a native number input produces today", () => {
    for (const raw of ["0", "1", "42", "100000000000", "0.5", "1234.5678", "1e9", "1.5e-3"]) {
      expect(value(raw)).toBe(Number(raw));
    }
  });

  it("keeps the eleven-zero limit intact", () => {
    expect(value("100000000000")).toBe(100000000000);
  });

  it("accepts exponent form and an explicit plus sign", () => {
    expect(value("1e9")).toBe(1_000_000_000);
    expect(value("1E9")).toBe(1_000_000_000);
    expect(value("+5")).toBe(5);
    expect(value("+2.5m")).toBe(2_500_000);
  });

  it("normalises negative zero", () => {
    expect(Object.is(value("-0"), 0)).toBe(true);
  });
});

describe("parseMagnitude — whitespace", () => {
  it("tolerates surrounding and pre-suffix whitespace", () => {
    expect(value("  1m  ")).toBe(1_000_000);
    expect(value("1 m")).toBe(1_000_000);
    expect(value("1M")).toBe(1_000_000);
    expect(value("\t2.5 b\n")).toBe(2_500_000_000);
  });
});

describe("parseMagnitude — negatives", () => {
  it("carries the sign through the scaling", () => {
    expect(value("-5")).toBe(-5);
    expect(value("-1m")).toBe(-1_000_000);
    expect(value("-2.5b")).toBe(-2_500_000_000);
  });
});

describe("parseMagnitude — thousands separators", () => {
  it("accepts well-formed en-US grouping", () => {
    expect(value("1,000")).toBe(1_000);
    expect(value("1,234,567")).toBe(1_234_567);
    expect(value("12,345")).toBe(12_345);
    expect(value("1,500.25")).toBe(1_500.25);
    expect(value("1,000k")).toBe(1_000_000);
  });

  it("rejects malformed grouping rather than silently producing garbage", () => {
    // `1,00` is a typo, and `1,5` is a European decimal comma this en-US field
    // must not guess at. Both are refused, not reinterpreted.
    for (const raw of ["1,00", "1,5", "1,0000", ",000", "1,2,3", "1,"]) {
      expect(rejection(raw)).toMatch(/separator|not a number/i);
    }
  });
});

describe("parseMagnitude — blank is not zero", () => {
  it("reports blank for empty and whitespace-only entries", () => {
    for (const raw of ["", "   ", "\t", "\n"]) {
      expect(parseMagnitude(raw)).toEqual({ kind: "blank" });
    }
  });

  it("never yields the number 0 for a blank entry", () => {
    const result = parseMagnitude("");
    expect(result.kind).not.toBe("ok");
  });
});

describe("parseMagnitude — invalid input is rejected, never coerced", () => {
  const invalid = [
    "abc",
    "1x",
    "1mm",
    "--5",
    "1.2.3",
    "-",
    "+",
    ".",
    "m",
    "k",
    "b",
    "1k2",
    "1..2",
    "1 2",
    "$1m",
    "1m%",
    "1e9k",
    "one million",
    "NaN",
    "Infinity",
    "0x10",
  ];

  it.each(invalid)("rejects %j", (raw) => {
    const result = parseMagnitude(raw);
    expect(result.kind).toBe("error");
  });

  it("never returns NaN or a coerced 0 for invalid input", () => {
    for (const raw of invalid) {
      const result = parseMagnitude(raw);
      // The union carries no `value` on error, so a caller physically cannot
      // read a number out of a rejection — assert the shape holds.
      expect(result).not.toHaveProperty("value");
    }
  });

  it("explains what was wrong, specifically", () => {
    expect(rejection("1x")).toContain("k = thousand");
    expect(rejection("1mm")).toContain("k = thousand");
    expect(rejection("1.2.3")).toContain("decimal point");
    expect(rejection("1,00")).toContain("separator");
    expect(rejection("1e9k")).toContain("exponent");
  });

  it("rejects overflow to infinity", () => {
    expect(rejection("1e999")).toContain("too large");
  });
});

describe("formatMagnitudeInput", () => {
  it("round-trips a committed value back through the parser", () => {
    for (const raw of ["1b", "5.5m", "0.5k", "-2.5b", "100000000000", "0"]) {
      const parsed = value(raw);
      expect(value(formatMagnitudeInput(parsed))).toBe(parsed);
    }
  });

  it("renders null as blank", () => {
    expect(formatMagnitudeInput(null)).toBe("");
  });
});

describe("formatMagnitudeEcho", () => {
  it("groups the resolved digits so an order of magnitude is countable", () => {
    expect(formatMagnitudeEcho(1_000_000_000)).toBe("1,000,000,000");
    expect(formatMagnitudeEcho(5_500_000)).toBe("5,500,000");
    expect(formatMagnitudeEcho(100_000_000_000)).toBe("100,000,000,000");
  });

  it("never abbreviates or uses exponent form", () => {
    expect(formatMagnitudeEcho(1e21)).not.toContain("e");
    expect(formatMagnitudeEcho(1_000_000)).not.toMatch(/[a-zA-Z]/);
  });

  it("keeps fractional values legible", () => {
    expect(formatMagnitudeEcho(1_500.25)).toBe("1,500.25");
    expect(formatMagnitudeEcho(0.5)).toBe("0.5");
  });
});
