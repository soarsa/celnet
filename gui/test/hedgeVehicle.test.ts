/**
 * The hedge-VEHICLE domain library: registry validation, the exit-mode resolution, and —
 * the one that carries real risk of being read backwards — the RESIDUAL wording.
 *
 * `residualDv01` is `target − hedged`, so POSITIVE means the rounding left risk on the
 * book and NEGATIVE means it took off too much. A desk reading that sign the wrong way
 * round mis-manages the position, so the sentence is pinned here rather than left to the
 * component.
 */
import { describe, expect, it } from "vitest";

import type { HedgeExitModeBinding, HedgeVehiclePlan, HedgeVehicleRule } from "../src/data/contract";
import {
  DURATION_PROXY_WARNING,
  actionUsesVehicle,
  isDurationProxy,
  matchLabel,
  maturityBucketLabel,
  namedVehicleIsRegistered,
  newHedgeVehicleRule,
  registeredHedgeInstruments,
  residualWording,
  resolveExitMode,
  validateExitModeBinding,
  validateHedgeVehicleRegistry,
  validateHedgeVehicleRule,
  vehicleNamesInstrument,
} from "../src/lib/hedgeVehicle";

const good: HedgeVehicleRule = {
  id: "us-corp-7-10y",
  instrumentId: "",
  product: "BOND",
  ccy: "USD",
  minMaturityYears: 7,
  maxMaturityYears: 10,
  hedgeInstrumentId: "TY-DEC26",
  isFuture: true,
  dv01PerUnit: 78,
  unitLabel: "contract",
};

const plan = (residualDv01: number, durationCorrect = true): HedgeVehiclePlan => ({
  hedgeInstrumentId: "TY-DEC26",
  unitLabel: "contract",
  wholeUnits: true,
  dv01Basis: durationCorrect ? "analytic" : "exposure-proxy",
  durationCorrect,
  targetDv01: 24_840,
  dv01PerUnit: 78,
  exactUnits: 318.46,
  units: 318,
  hedgedDv01: 24_804,
  residualDv01,
  summary: "Sell 318 contracts of TY-DEC26",
});

describe("which leaves carry a vehicle", () => {
  it("every SIZE-BEARING action does", () => {
    for (const k of ["submit_market_order", "rfq_out", "split", "clear_risk", "cross_internal"] as const) {
      expect(actionUsesVehicle(k)).toBe(true);
    }
  });
  it("the actions that place NO order do not — asking what they hedge with is meaningless", () => {
    for (const k of ["warehouse", "skew", "escalate"] as const) {
      expect(actionUsesVehicle(k)).toBe(false);
    }
  });
  it("only the NAMED kinds carry an instrument", () => {
    expect(vehicleNamesInstrument("instrument")).toBe(true);
    expect(vehicleNamesInstrument("future")).toBe(true);
    expect(vehicleNamesInstrument("self")).toBe(false);
    expect(vehicleNamesInstrument("benchmark")).toBe(false);
  });
});

describe("registry validation surfaces EVERY problem at once", () => {
  it("accepts a well-formed row", () => {
    expect(validateHedgeVehicleRule(good, [])).toEqual([]);
  });
  it("rejects a zero or negative DV01 per unit — it cannot size a hedge", () => {
    expect(validateHedgeVehicleRule({ ...good, dv01PerUnit: 0 }, []).join(" ")).toMatch(/DV01 per unit/);
    expect(validateHedgeVehicleRule({ ...good, dv01PerUnit: -1 }, []).join(" ")).toMatch(/DV01 per unit/);
  });
  it("rejects an inverted or degenerate maturity bucket", () => {
    expect(validateHedgeVehicleRule({ ...good, minMaturityYears: 10, maxMaturityYears: 7 }, []).join(" ")).toMatch(
      /Max maturity/,
    );
    expect(validateHedgeVehicleRule({ ...good, minMaturityYears: 7, maxMaturityYears: 7 }, []).join(" ")).toMatch(
      /Max maturity/,
    );
  });
  it("allows an UNSET bucket (both bounds 0 ⇒ any maturity)", () => {
    expect(validateHedgeVehicleRule({ ...good, minMaturityYears: 0, maxMaturityYears: 0 }, [])).toEqual([]);
  });
  it("rejects an empty hedge instrument and an empty id", () => {
    expect(validateHedgeVehicleRule({ ...good, hedgeInstrumentId: "  " }, []).join(" ")).toMatch(
      /Hedge instrument is required/,
    );
    expect(validateHedgeVehicleRule({ ...good, id: "" }, []).join(" ")).toMatch(/Id is required/);
  });
  it("rejects a duplicate id", () => {
    expect(validateHedgeVehicleRule(good, ["us-corp-7-10y"]).join(" ")).toMatch(/already used/);
  });
  it("returns ALL problems together rather than stopping at the first", () => {
    const broken: HedgeVehicleRule = {
      ...good,
      id: "",
      hedgeInstrumentId: "",
      dv01PerUnit: 0,
      minMaturityYears: 10,
      maxMaturityYears: 2,
      unitLabel: "",
    };
    expect(validateHedgeVehicleRule(broken, []).length).toBeGreaterThanOrEqual(5);
  });
  it("validates the whole roster, attributing each problem to its row", () => {
    const errors = validateHedgeVehicleRegistry([good, { ...good, dv01PerUnit: 0 }]);
    // The duplicate id AND the bad DV01 are both reported, each prefixed by the row.
    expect(errors.join(" ")).toMatch(/us-corp-7-10y/);
    expect(errors.length).toBeGreaterThanOrEqual(2);
  });
  it("a fresh row defaults to a cash 1mm-face vehicle", () => {
    const fresh = newHedgeVehicleRule("vehicle-1");
    expect(fresh.isFuture).toBe(false);
    expect(fresh.unitLabel).toBe("1mm face");
  });
});

describe("a NAMED vehicle must be one the registry can price", () => {
  it("lists the registry's hedge instruments, deduped and sorted", () => {
    expect(
      registeredHedgeInstruments([good, { ...good, id: "b", hedgeInstrumentId: "FV-DEC26" }, { ...good, id: "c" }]),
    ).toEqual(["FV-DEC26", "TY-DEC26"]);
  });
  it("accepts self / benchmark unconditionally (they name nothing)", () => {
    expect(namedVehicleIsRegistered("self", "", [])).toBe(true);
    expect(namedVehicleIsRegistered("benchmark", "", [])).toBe(true);
  });
  it("rejects a named vehicle the registry does not carry", () => {
    expect(namedVehicleIsRegistered("future", "TY-DEC26", [good])).toBe(true);
    expect(namedVehicleIsRegistered("future", "UB-DEC26", [good])).toBe(false);
    expect(namedVehicleIsRegistered("instrument", "", [good])).toBe(false);
  });
});

describe("display helpers", () => {
  it("renders an unset bucket as 'any' and a set one as a range", () => {
    expect(maturityBucketLabel({ ...good, minMaturityYears: 0, maxMaturityYears: 0 })).toBe("any");
    expect(maturityBucketLabel(good)).toBe("7–10y");
  });
  it("renders each unset match axis as 'any …'", () => {
    expect(matchLabel(good)).toBe("any instrument · BOND · USD");
  });
});

describe("residual wording spells the SIGN out", () => {
  it("a POSITIVE residual means rounded down — risk still on the book", () => {
    expect(residualWording(plan(36))).toBe("rounded down, 36 DV01 still on the book");
  });
  it("a NEGATIVE residual means rounded up — the book is over-hedged", () => {
    expect(residualWording(plan(-31))).toBe("rounded up, over-hedged by 31 DV01");
  });
  it("a zero residual reads as exact", () => {
    expect(residualWording(plan(0))).toBe("exact — no residual");
  });
});

describe("the duration-proxy honesty flag", () => {
  it("flags a plan sized off the duration-blind exposure proxy", () => {
    expect(isDurationProxy(plan(36, false))).toBe(true);
  });
  it("does NOT flag a duration-correct plan", () => {
    expect(isDurationProxy(plan(36, true))).toBe(false);
  });
  it("the warning says the size is approximate, not exact", () => {
    expect(DURATION_PROXY_WARNING).toMatch(/[Aa]pproximate/);
    expect(DURATION_PROXY_WARNING).toMatch(/duration-blind/);
  });
});

describe("exit-mode resolution is most-specific-wins", () => {
  const bindings: HedgeExitModeBinding[] = [
    { scopeKind: "desk", scopeId: "emea", mode: "suggest" },
    { scopeKind: "book", scopeId: "fi-rates-emea", mode: "auto" },
    { scopeKind: "instrument", scopeId: "US10Y", mode: "suggest" },
  ];
  it("instrument beats book beats desk", () => {
    expect(resolveExitMode(bindings, { instrument: "US10Y", book: "fi-rates-emea", desk: "emea" })).toBe("suggest");
    expect(resolveExitMode(bindings, { instrument: "UK5Y", book: "fi-rates-emea", desk: "emea" })).toBe("auto");
    expect(resolveExitMode(bindings, { instrument: "UK5Y", book: "fi-other", desk: "emea" })).toBe("suggest");
  });
  it("an UNBOUND scope is auto — the pre-feature behaviour", () => {
    expect(resolveExitMode([], { book: "anything" })).toBe("auto");
    expect(resolveExitMode(bindings, { book: "fi-other", desk: "apac" })).toBe("auto");
  });
  it("rejects an empty or duplicate binding scope", () => {
    expect(validateExitModeBinding({ scopeKind: "book", scopeId: " ", mode: "auto" }, []).join(" ")).toMatch(
      /Scope id is required/,
    );
    expect(
      validateExitModeBinding({ scopeKind: "book", scopeId: "fi-rates-emea", mode: "suggest" }, bindings).join(" "),
    ).toMatch(/already exists/);
  });
});
